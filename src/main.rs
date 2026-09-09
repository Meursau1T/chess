use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use chess_tui::ai::discover_pikafish;
use chess_tui::app::{App, AppOptions, Difficulty};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

#[derive(Debug)]
struct Cli {
    engine: Option<PathBuf>,
    think_time: Duration,
    difficulty: Difficulty,
    emoji_pieces: bool,
}

fn main() -> anyhow::Result<()> {
    let Some(cli) = parse_args()? else {
        return Ok(());
    };
    let engine_path = discover_pikafish(cli.engine.as_deref());

    enable_raw_mode()?;
    let mut output = stdout();
    execute!(output, EnterAlternateScreen, EnableBracketedPaste)?;
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(output);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(AppOptions {
        engine_path,
        think_time: cli.think_time,
        difficulty: cli.difficulty,
        emoji_pieces: cli.emoji_pieces,
    });
    app.run(&mut terminal)?;
    terminal.show_cursor()?;
    Ok(())
}

fn parse_args() -> anyhow::Result<Option<Cli>> {
    let mut engine = None;
    let mut think_time = Duration::from_millis(1_200);
    let mut difficulty = Difficulty::default();
    let mut emoji_pieces = false;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--engine" | "-e" => {
                let path = args.next().context("--engine 后需要填写 Pikafish 路径")?;
                engine = Some(PathBuf::from(path));
            }
            "--think-ms" => {
                let value = args.next().context("--think-ms 后需要填写毫秒数")?;
                let milliseconds: u64 = value.parse().context("--think-ms 必须是正整数")?;
                think_time = Duration::from_millis(milliseconds.max(50));
            }
            "--difficulty" | "-d" => {
                let value = args.next().context("--difficulty 后需要填写 1 到 20")?;
                difficulty = Difficulty::parse(&value).map_err(anyhow::Error::msg)?;
            }
            "--text" => emoji_pieces = false,
            "--emoji" => emoji_pieces = true,
            "--help" | "-h" => {
                print_help();
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("chess_tui {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            unknown => anyhow::bail!("未知参数 {unknown}，使用 --help 查看帮助"),
        }
    }
    Ok(Some(Cli {
        engine,
        think_time,
        difficulty,
        emoji_pieces,
    }))
}

fn print_help() {
    println!(
        "chess_tui - 终端中国象棋\n\n\
         用法  chess_tui [选项]\n\n\
         -e, --engine PATH   指定 Pikafish 可执行文件\n\
             --think-ms N   AI 每步思考毫秒数，默认 1200\n\
         -d, --difficulty N AI 难度 1 到 20，默认 20\n\
             --text         使用中文棋子，默认开启\n\
             --emoji        改用 Unicode 象棋棋子\n\
         -h, --help          显示帮助\n\n\
         也可通过 PIKAFISH_PATH 环境变量指定引擎。"
    );
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste);
    }
}
