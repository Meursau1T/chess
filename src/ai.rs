use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chessai::{Engine, Limits};
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPurpose {
    AiMove,
    Evaluation,
    Hint,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub id: u64,
    pub fen: String,
    pub movetime: Duration,
    pub purpose: SearchPurpose,
    pub multi_pv: u32,
}

#[derive(Debug, Clone, Default)]
pub struct AnalysisInfo {
    pub depth: Option<u32>,
    pub seldepth: Option<u32>,
    pub multipv: u32,
    pub score_cp: Option<i32>,
    pub mate: Option<i32>,
    pub time_ms: Option<u64>,
    pub nodes: Option<u64>,
    pub nps: Option<u64>,
    pub pv: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum AiEvent {
    Ready {
        name: String,
        external: bool,
    },
    Info {
        id: u64,
        purpose: SearchPurpose,
        info: AnalysisInfo,
    },
    BestMove {
        id: u64,
        purpose: SearchPurpose,
        best_move: Option<String>,
        ponder: Option<String>,
    },
    Error(String),
}

#[derive(Debug)]
enum AiCommand {
    Search(SearchRequest),
    Stop,
    Quit,
}

pub struct AiHandle {
    tx: Sender<AiCommand>,
    rx: Receiver<AiEvent>,
    worker: Option<JoinHandle<()>>,
}

impl AiHandle {
    pub fn start(engine_path: Option<PathBuf>) -> Self {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let worker = thread::spawn(move || {
            if let Some(path) = engine_path {
                match run_external(&path, &command_rx, &event_tx) {
                    Ok(()) => return,
                    Err(error) => {
                        let _ = event_tx.send(AiEvent::Error(format!(
                            "Pikafish 启动失败，改用内置引擎：{error}"
                        )));
                    }
                }
            }
            run_builtin(&command_rx, &event_tx);
        });
        Self {
            tx: command_tx,
            rx: event_rx,
            worker: Some(worker),
        }
    }

    pub fn search(&self, request: SearchRequest) {
        let _ = self.tx.send(AiCommand::Search(request));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(AiCommand::Stop);
    }

    pub fn try_recv(&self) -> Result<AiEvent, TryRecvError> {
        self.rx.try_recv()
    }
}

impl Drop for AiHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(AiCommand::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn discover_pikafish(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit
        && path.is_file()
    {
        return Some(path.to_path_buf());
    }

    if let Some(path) = env::var_os("PIKAFISH_PATH").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }

    if let Ok(cwd) = env::current_dir() {
        for relative in ["engine/pikafish", "engines/pikafish", "pikafish"] {
            let path = cwd.join(relative);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    let names: &[&str] = if cfg!(windows) {
        &["pikafish.exe", "pikafish"]
    } else {
        &["pikafish"]
    };
    if let Some(path_var) = env::var_os("PATH") {
        for directory in env::split_paths(&path_var) {
            for name in names {
                let path = directory.join(name);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
struct ActiveSearch {
    request: SearchRequest,
    canceled: bool,
}

fn run_external(
    path: &Path,
    commands: &Receiver<AiCommand>,
    events: &Sender<AiEvent>,
) -> anyhow::Result<()> {
    let mut command = Command::new(path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        command.current_dir(parent);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("无法连接引擎输入"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("无法连接引擎输出"))?;
    let (line_tx, line_rx) = unbounded::<String>();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    send_line(&mut stdin, "uci")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut engine_name = "Pikafish".to_string();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("等待 uciok 超时");
        }
        let line = line_rx
            .recv_timeout(remaining)
            .map_err(|_| anyhow::anyhow!("等待 uciok 超时"))?;
        if let Some(name) = line.strip_prefix("id name ") {
            engine_name = name.trim().to_string();
        }
        if line.trim() == "uciok" {
            break;
        }
    }

    let threads = thread::available_parallelism()
        .map(|value| value.get().clamp(1, 8))
        .unwrap_or(2);
    send_line(
        &mut stdin,
        &format!("setoption name Threads value {threads}"),
    )?;
    send_line(&mut stdin, "setoption name Hash value 256")?;
    send_line(&mut stdin, "setoption name UCI_ShowWDL value true")?;
    send_line(&mut stdin, "isready")?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("等待 readyok 超时");
        }
        let line = line_rx
            .recv_timeout(remaining)
            .map_err(|_| anyhow::anyhow!("等待 readyok 超时"))?;
        if line.trim() == "readyok" {
            break;
        }
    }
    let _ = events.send(AiEvent::Ready {
        name: engine_name,
        external: true,
    });

    let mut active: Option<ActiveSearch> = None;
    let mut pending: Option<SearchRequest> = None;
    let mut should_quit = false;

    while !should_quit {
        crossbeam_channel::select! {
            recv(commands) -> message => {
                let Ok(message) = message else { break };
                match message {
                    AiCommand::Search(request) => {
                        if let Some(search) = active.as_mut() {
                            search.canceled = true;
                            pending = Some(request);
                            send_line(&mut stdin, "stop")?;
                        } else {
                            start_external_search(&mut stdin, &request)?;
                            active = Some(ActiveSearch { request, canceled: false });
                        }
                    }
                    AiCommand::Stop => {
                        pending = None;
                        if let Some(search) = active.as_mut() {
                            search.canceled = true;
                            send_line(&mut stdin, "stop")?;
                        }
                    }
                    AiCommand::Quit => {
                        if active.is_some() {
                            let _ = send_line(&mut stdin, "stop");
                        }
                        let _ = send_line(&mut stdin, "quit");
                        should_quit = true;
                    }
                }
            }
            recv(line_rx) -> message => {
                let Ok(line) = message else {
                    if !should_quit {
                        anyhow::bail!("引擎进程已退出");
                    }
                    break;
                };
                if line.starts_with("info ") {
                    if let Some(search) = active.as_ref()
                        && !search.canceled
                        && let Some(info) = parse_info_line(&line)
                    {
                        let _ = events.send(AiEvent::Info {
                            id: search.request.id,
                            purpose: search.request.purpose,
                            info,
                        });
                    }
                } else if line.starts_with("bestmove") {
                    if let Some(search) = active.take()
                        && !search.canceled
                    {
                        let (best_move, ponder) = parse_bestmove_line(&line);
                        let _ = events.send(AiEvent::BestMove {
                            id: search.request.id,
                            purpose: search.request.purpose,
                            best_move,
                            ponder,
                        });
                    }
                    if let Some(request) = pending.take() {
                        start_external_search(&mut stdin, &request)?;
                        active = Some(ActiveSearch { request, canceled: false });
                    }
                }
            }
        }
    }

    let _ = child.wait();
    let _ = reader.join();
    Ok(())
}

fn start_external_search(stdin: &mut ChildStdin, request: &SearchRequest) -> anyhow::Result<()> {
    send_line(
        stdin,
        &format!("setoption name MultiPV value {}", request.multi_pv.max(1)),
    )?;
    send_line(stdin, &format!("position fen {}", request.fen))?;
    send_line(
        stdin,
        &format!("go movetime {}", request.movetime.as_millis().max(1)),
    )
}

fn send_line(stdin: &mut ChildStdin, command: &str) -> anyhow::Result<()> {
    writeln!(stdin, "{command}")?;
    stdin.flush()?;
    Ok(())
}

fn run_builtin(commands: &Receiver<AiCommand>, events: &Sender<AiEvent>) {
    let thread_count = thread::available_parallelism()
        .map(|value| value.get().clamp(1, 4) as u8)
        .unwrap_or(1);
    let mut engine = Engine::builder()
        .hash_size(64)
        .threads(thread_count)
        .use_book(true)
        .build();
    let _ = events.send(AiEvent::Ready {
        name: "内置轻量引擎".to_string(),
        external: false,
    });

    while let Ok(first) = commands.recv() {
        let mut command = first;
        while let Ok(next) = commands.try_recv() {
            command = next;
        }
        match command {
            AiCommand::Quit => break,
            AiCommand::Stop => continue,
            AiCommand::Search(request) => {
                if let Err(error) = engine.set_fen(&request.fen) {
                    let _ = events.send(AiEvent::Error(format!("无法载入局面：{error}")));
                    continue;
                }
                let result = engine.search_with(Limits::new().time(request.movetime), |snapshot| {
                    let info = AnalysisInfo {
                        depth: Some(snapshot.depth as u32),
                        seldepth: None,
                        multipv: 1,
                        score_cp: Some(snapshot.score),
                        mate: None,
                        time_ms: Some(snapshot.time.as_millis() as u64),
                        nodes: Some(snapshot.nodes),
                        nps: Some(snapshot.nps),
                        pv: snapshot
                            .pv
                            .iter()
                            .map(|mv| mv.to_iccs().replace('-', ""))
                            .collect(),
                    };
                    let _ = events.send(AiEvent::Info {
                        id: request.id,
                        purpose: request.purpose,
                        info,
                    });
                });
                let _ = events.send(AiEvent::BestMove {
                    id: request.id,
                    purpose: request.purpose,
                    best_move: result.best_move.map(|mv| mv.to_iccs().replace('-', "")),
                    ponder: None,
                });
            }
        }
    }
}

fn parse_bestmove_line(line: &str) -> (Option<String>, Option<String>) {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let best = tokens.get(1).and_then(|value| {
        if matches!(*value, "(none)" | "none" | "0000") {
            None
        } else {
            Some((*value).to_string())
        }
    });
    let ponder = tokens
        .windows(2)
        .find(|pair| pair[0] == "ponder")
        .map(|pair| pair[1].to_string());
    (best, ponder)
}

pub fn parse_info_line(line: &str) -> Option<AnalysisInfo> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.first().copied() != Some("info") {
        return None;
    }
    let mut info = AnalysisInfo {
        multipv: 1,
        ..AnalysisInfo::default()
    };
    let mut index = 1;
    while index < tokens.len() {
        match tokens[index] {
            "depth" => read_u32(&tokens, &mut index, &mut info.depth),
            "seldepth" => read_u32(&tokens, &mut index, &mut info.seldepth),
            "multipv" => {
                let mut value = None;
                read_u32(&tokens, &mut index, &mut value);
                info.multipv = value.unwrap_or(1);
            }
            "time" => read_u64(&tokens, &mut index, &mut info.time_ms),
            "nodes" => read_u64(&tokens, &mut index, &mut info.nodes),
            "nps" => read_u64(&tokens, &mut index, &mut info.nps),
            "score" if index + 2 < tokens.len() => {
                match tokens[index + 1] {
                    "cp" => info.score_cp = tokens[index + 2].parse().ok(),
                    "mate" => info.mate = tokens[index + 2].parse().ok(),
                    _ => {}
                }
                index += 3;
            }
            "pv" => {
                info.pv = tokens[index + 1..]
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect();
                break;
            }
            _ => index += 1,
        }
    }
    if info.depth.is_none() && info.pv.is_empty() && info.score_cp.is_none() && info.mate.is_none()
    {
        None
    } else {
        Some(info)
    }
}

fn read_u32(tokens: &[&str], index: &mut usize, target: &mut Option<u32>) {
    *target = tokens.get(*index + 1).and_then(|value| value.parse().ok());
    *index += 2;
}

fn read_u64(tokens: &[&str], index: &mut usize, target: &mut Option<u64>) {
    *target = tokens.get(*index + 1).and_then(|value| value.parse().ok());
    *index += 2;
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn parses_standard_info_line() {
        let info = parse_info_line(
            "info depth 18 seldepth 27 multipv 2 score cp -34 nodes 123456 nps 999000 time 345 pv h2e2 h9g7",
        )
        .unwrap();
        assert_eq!(info.depth, Some(18));
        assert_eq!(info.seldepth, Some(27));
        assert_eq!(info.multipv, 2);
        assert_eq!(info.score_cp, Some(-34));
        assert_eq!(info.nodes, Some(123_456));
        assert_eq!(info.pv, ["h2e2", "h9g7"]);
    }

    #[test]
    fn parses_mate_and_bestmove() {
        let info = parse_info_line("info depth 12 score mate 3 pv e0e1").unwrap();
        assert_eq!(info.mate, Some(3));
        let (best, ponder) = parse_bestmove_line("bestmove h2e2 ponder h9g7");
        assert_eq!(best.as_deref(), Some("h2e2"));
        assert_eq!(ponder.as_deref(), Some("h9g7"));
    }

    #[test]
    fn external_pikafish_works_when_test_path_is_set() {
        let Some(path) = std::env::var_os("TEST_PIKAFISH_PATH").map(PathBuf::from) else {
            return;
        };
        let handle = AiHandle::start(Some(path));
        handle.search(SearchRequest {
            id: 7,
            fen: "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1"
                .to_string(),
            movetime: Duration::from_millis(100),
            purpose: SearchPurpose::Hint,
            multi_pv: 3,
        });

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut external_ready = false;
        let mut best_move = None;
        while Instant::now() < deadline && best_move.is_none() {
            match handle.try_recv() {
                Ok(AiEvent::Ready { external, .. }) => external_ready = external,
                Ok(AiEvent::BestMove {
                    best_move: best, ..
                }) => best_move = best,
                Ok(AiEvent::Error(error)) => panic!("external engine error: {error}"),
                Ok(AiEvent::Info { .. }) | Err(TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(TryRecvError::Disconnected) => break,
            }
        }
        assert!(external_ready);
        assert!(best_move.is_some());
    }
}
