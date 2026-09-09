use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chessai::{Color, Move, Square};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::Backend};

use crate::ai::{AiEvent, AiHandle, AnalysisInfo, SearchPurpose, SearchRequest};
use crate::game::{Game, PositionState, color_name};
use crate::{notation, ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Board,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanMode {
    Side(Color),
    Both,
}

impl HumanMode {
    pub fn is_human_turn(self, side: Color) -> bool {
        match self {
            Self::Side(human) => human == side,
            Self::Both => true,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Side(Color::Red) => "你执红",
            Self::Side(Color::Black) => "你执黑",
            Self::Both => "双人对弈",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Difficulty(u8);

impl Difficulty {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 20;

    pub const fn new(level: u8) -> Option<Self> {
        if level >= Self::MIN && level <= Self::MAX {
            Some(Self(level))
        } else {
            None
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let level = value
            .parse::<u8>()
            .map_err(|_| "难度必须是 1 到 20 的整数".to_string())?;
        Self::new(level).ok_or_else(|| "难度必须是 1 到 20 的整数".to_string())
    }

    pub const fn level(self) -> u8 {
        self.0
    }

    pub const fn is_full_strength(self) -> bool {
        self.0 == Self::MAX
    }

    pub const fn selection_depth(self) -> Option<u32> {
        if self.is_full_strength() {
            None
        } else {
            Some(self.0 as u32 + 1)
        }
    }

    pub const fn weakness(self) -> Option<u8> {
        if self.is_full_strength() {
            None
        } else {
            Some(120 - 2 * self.0)
        }
    }
}

impl Default for Difficulty {
    fn default() -> Self {
        Self(Self::MAX)
    }
}

#[derive(Debug, Clone)]
pub struct AnalysisLine {
    pub info: AnalysisInfo,
    pub score_cp_red: Option<i32>,
    pub mate_red: Option<i32>,
    pub pv_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionEvaluation {
    pub score_cp_red: Option<i32>,
    pub mate_red: Option<i32>,
    pub depth: u32,
}

#[derive(Debug, Clone, Copy)]
struct ActiveSearch {
    id: u64,
    purpose: SearchPurpose,
    side: Color,
    position_index: usize,
    skill_move: Option<Move>,
}

pub struct AppOptions {
    pub engine_path: Option<PathBuf>,
    pub think_time: Duration,
    pub difficulty: Difficulty,
    pub emoji_pieces: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            engine_path: None,
            think_time: Duration::from_millis(1_200),
            difficulty: Difficulty::default(),
            emoji_pieces: false,
        }
    }
}

pub struct App {
    pub(crate) game: Game,
    pub(crate) input: String,
    pub(crate) focus: Focus,
    pub(crate) cursor: Square,
    pub(crate) selected: Option<Square>,
    pub(crate) selected_moves: Vec<Move>,
    pub(crate) flipped: bool,
    pub(crate) message: String,
    pub(crate) human_mode: HumanMode,
    pub(crate) thinking: bool,
    pub(crate) engine_name: String,
    pub(crate) external_engine: bool,
    pub(crate) analysis_lines: BTreeMap<u32, AnalysisLine>,
    pub(crate) position_evaluations: Vec<Option<PositionEvaluation>>,
    pub(crate) hint_move: Option<Move>,
    pub(crate) emoji_pieces: bool,
    pub(crate) think_time: Duration,
    pub(crate) difficulty: Difficulty,
    input_history: Vec<String>,
    history_index: Option<usize>,
    ai: AiHandle,
    active_search: Option<ActiveSearch>,
    search_sequence: u64,
    random_state: u64,
    quit: bool,
}

impl App {
    pub fn new(options: AppOptions) -> Self {
        Self {
            game: Game::new(),
            input: String::new(),
            focus: Focus::Input,
            cursor: Square::from_iccs("e0").expect("valid initial cursor"),
            selected: None,
            selected_moves: Vec::new(),
            flipped: false,
            message: "输入 马二进三 后按 Enter，Tab 可查看棋盘走法".to_string(),
            human_mode: HumanMode::Side(Color::Red),
            thinking: false,
            engine_name: "正在启动引擎".to_string(),
            external_engine: false,
            analysis_lines: BTreeMap::new(),
            position_evaluations: vec![None],
            hint_move: None,
            emoji_pieces: options.emoji_pieces,
            think_time: options.think_time,
            difficulty: options.difficulty,
            input_history: Vec::new(),
            history_index: None,
            ai: AiHandle::start(options.engine_path),
            active_search: None,
            search_sequence: 0,
            random_state: random_seed(),
            quit: false,
        }
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()> {
        while !self.quit {
            self.poll_ai();
            terminal.draw(|frame| ui::draw(frame, self))?;
            if event::poll(Duration::from_millis(50))? {
                let event = event::read()?;
                self.handle_event(event);
            }
        }
        self.ai.stop();
        Ok(())
    }

    pub(crate) fn is_evaluating(&self) -> bool {
        self.active_search
            .is_some_and(|search| search.purpose == SearchPurpose::Evaluation)
    }

    fn search_blocks_moves(&self) -> bool {
        self.thinking && !self.is_evaluating()
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
            Event::Paste(text) if self.focus == Focus::Input => self.input.push_str(&text),
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if key.code == KeyCode::Tab {
            self.focus = match self.focus {
                Focus::Input => Focus::Board,
                Focus::Board => Focus::Input,
            };
            return;
        }

        match self.focus {
            Focus::Input => self.handle_input_key(key),
            Focus::Board => self.handle_board_key(key),
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => {
                self.input.pop();
                self.history_index = None;
            }
            KeyCode::Esc => {
                self.input.clear();
                self.history_index = None;
            }
            KeyCode::Up => self.previous_input(),
            KeyCode::Down => self.next_input(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.input.push(character);
                self.history_index = None;
            }
            _ => {}
        }
    }

    fn handle_board_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(0, 1),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(0, -1),
            KeyCode::Left | KeyCode::Char('h') => self.move_cursor(-1, 0),
            KeyCode::Right | KeyCode::Char('l') => self.move_cursor(1, 0),
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_board_square(),
            KeyCode::Esc => self.clear_selection(),
            KeyCode::Char(':') | KeyCode::Char('/') => {
                self.focus = Focus::Input;
                if key.code == KeyCode::Char('/') {
                    self.input.push('/');
                }
            }
            _ => {}
        }
    }

    fn move_cursor(&mut self, display_dx: i8, display_dy: i8) {
        let (dx, dy) = if self.flipped {
            (-display_dx, -display_dy)
        } else {
            (display_dx, display_dy)
        };
        let file = (self.cursor.file() as i8 + dx).clamp(0, 8) as u8;
        let rank = (self.cursor.rank() as i8 + dy).clamp(0, 9) as u8;
        self.cursor = Square::from_rank_file(rank, file).expect("cursor remains on board");
    }

    fn activate_board_square(&mut self) {
        if self.search_blocks_moves() {
            self.message = "电脑正在思考".to_string();
            return;
        }
        let side = self.game.side_to_move();
        if !self.human_mode.is_human_turn(side) {
            self.message = format!("现在由{}走棋", color_name(side));
            return;
        }

        if let Some(selected) = self.selected {
            if selected == self.cursor {
                self.clear_selection();
                return;
            }
            if let Some(mv) = self
                .selected_moves
                .iter()
                .find(|mv| mv.dst() == self.cursor)
                .copied()
            {
                self.clear_selection();
                self.play_move(mv);
                return;
            }
        }

        if self
            .game
            .piece_at(self.cursor)
            .is_some_and(|piece| piece.color() == side)
        {
            self.selected = Some(self.cursor);
            self.selected_moves = self.game.legal_moves_from(self.cursor);
            self.message = if self.selected_moves.is_empty() {
                "这枚棋子当前不能走".to_string()
            } else {
                format!("有 {} 个合法落点", self.selected_moves.len())
            };
        } else {
            self.message = "这里没有当前一方的棋子".to_string();
        }
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_moves.clear();
    }

    fn submit_input(&mut self) {
        let command = self.input.trim().to_string();
        self.input.clear();
        self.history_index = None;
        if command.is_empty() {
            return;
        }
        if self.input_history.last() != Some(&command) {
            self.input_history.push(command.clone());
        }
        self.execute_command(&command);
    }

    fn execute_command(&mut self, command: &str) {
        let compact: String = command.chars().filter(|c| !c.is_whitespace()).collect();
        let lower = compact.to_ascii_lowercase();
        if let Some(argument) = difficulty_argument(&lower) {
            self.handle_difficulty_command(argument);
            return;
        }
        match lower.as_str() {
            "新局" | "新棋" | "new" | "/new" => self.new_game(),
            "悔棋" | "undo" | "/undo" => self.undo_for_human(),
            "提示" | "hint" | "/hint" => self.start_search(SearchPurpose::Hint, self.think_time),
            "分析" | "analyze" | "/analyze" => {
                self.start_search(SearchPurpose::Hint, self.think_time.saturating_mul(3));
            }
            "停止" | "stop" | "/stop" => self.cancel_search("已停止思考"),
            "翻转" | "rotate" | "/rotate" => {
                self.flipped = !self.flipped;
                self.message = "棋盘已翻转".to_string();
            }
            "执红" | "红方" | "/red" => self.set_human_mode(HumanMode::Side(Color::Red)),
            "执黑" | "黑方" | "/black" => self.set_human_mode(HumanMode::Side(Color::Black)),
            "双人" | "/both" => self.set_human_mode(HumanMode::Both),
            "中文棋子" | "/text" => {
                self.emoji_pieces = false;
                self.message = "已切换为中文棋子".to_string();
            }
            "emoji" | "表情棋子" | "/emoji" => {
                self.emoji_pieces = true;
                self.message = "已切换为 Unicode 象棋棋子".to_string();
            }
            "帮助" | "help" | "/help" => {
                self.message = "着法示例 马二进三；命令 新局 悔棋 提示 分析 停止 翻转 执红 执黑 双人 难度1-20 退出".to_string();
            }
            "退出" | "quit" | "exit" | "/quit" | "/exit" => self.quit = true,
            _ if lower.starts_with("fen:") || lower.starts_with("fen：") => {
                let fen = command
                    .split_once([':', '：'])
                    .map(|(_, value)| value.trim())
                    .unwrap_or_default();
                self.load_fen(fen);
            }
            _ => self.play_notation(command),
        }
    }

    fn handle_difficulty_command(&mut self, argument: &str) {
        let value = argument.trim_start_matches([':', '：', '=']);
        if value.is_empty() {
            self.message = difficulty_message(self.difficulty, true);
            return;
        }
        let difficulty = match Difficulty::parse(value) {
            Ok(difficulty) => difficulty,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if difficulty == self.difficulty {
            self.message = difficulty_message(difficulty, true);
            return;
        }
        self.invalidate_search();
        self.difficulty = difficulty;
        self.message = difficulty_message(difficulty, false);
        self.schedule_next_search();
    }

    fn new_game(&mut self) {
        self.invalidate_search();
        self.game.reset();
        self.clear_selection();
        self.hint_move = None;
        self.analysis_lines.clear();
        self.position_evaluations.clear();
        self.position_evaluations.push(None);
        self.message = "新的一局，红方先行".to_string();
        self.schedule_next_search();
    }

    fn load_fen(&mut self, fen: &str) {
        self.invalidate_search();
        match self.game.set_fen(fen) {
            Ok(()) => {
                self.clear_selection();
                self.hint_move = None;
                self.analysis_lines.clear();
                self.position_evaluations.clear();
                self.position_evaluations.push(None);
                self.message = "局面已载入".to_string();
                self.schedule_next_search();
            }
            Err(error) => self.message = format!("FEN 无效：{error}"),
        }
    }

    fn set_human_mode(&mut self, mode: HumanMode) {
        self.invalidate_search();
        self.human_mode = mode;
        self.message = format!("已切换为 {}", mode.label());
        self.schedule_next_search();
    }

    fn play_notation(&mut self, input: &str) {
        if self.search_blocks_moves() {
            self.message = "电脑正在思考，输入 停止 可中止".to_string();
            return;
        }
        let side = self.game.side_to_move();
        if !self.human_mode.is_human_turn(side) {
            self.message = format!("现在由{}电脑走棋", color_name(side));
            return;
        }
        let legal = self.game.legal_moves();
        match notation::parse_move(input, self.game.position(), &legal) {
            Ok(mv) => self.play_move(mv),
            Err(error) => self.message = error.to_string(),
        }
    }

    fn play_move(&mut self, mv: Move) {
        self.invalidate_search();
        match self.game.play(mv) {
            Ok(record) => {
                self.prepare_current_evaluation();
                self.cursor = mv.dst();
                self.clear_selection();
                self.hint_move = None;
                self.analysis_lines.clear();
                self.after_move(record.notation);
            }
            Err(error) => self.message = error.to_string(),
        }
    }

    fn after_move(&mut self, notation: String) {
        match self.game.state() {
            PositionState::Won {
                winner,
                by_checkmate,
            } => {
                let mate_red = if winner == Color::Red { 1 } else { -1 };
                self.update_position_evaluation(self.game.history().len(), 0, None, Some(mate_red));
                self.message = format!(
                    "{}，{}获胜",
                    if by_checkmate { "将死" } else { "困毙" },
                    color_name(winner)
                );
                self.thinking = false;
            }
            PositionState::Check(side) => {
                self.message = format!("{notation}，{}被将军", color_name(side));
                self.schedule_next_search();
            }
            PositionState::Playing => {
                self.message = notation;
                self.schedule_next_search();
            }
        }
    }

    fn undo_for_human(&mut self) {
        self.invalidate_search();
        let Some(record) = self.game.undo() else {
            self.message = "还没有棋可悔".to_string();
            self.schedule_next_search();
            return;
        };
        if let HumanMode::Side(human) = self.human_mode {
            while self.game.side_to_move() != human && !self.game.history().is_empty() {
                self.game.undo();
            }
        }
        self.position_evaluations
            .truncate(self.game.history().len() + 1);
        self.clear_selection();
        self.hint_move = None;
        self.analysis_lines.clear();
        self.message = format!("已撤销 {}", record.notation);
        self.schedule_next_search();
    }

    fn prepare_current_evaluation(&mut self) {
        let required = self.game.history().len() + 1;
        self.position_evaluations.truncate(required);
        self.position_evaluations.resize(required, None);
    }

    fn update_position_evaluation(
        &mut self,
        index: usize,
        depth: u32,
        score_cp_red: Option<i32>,
        mate_red: Option<i32>,
    ) {
        if score_cp_red.is_none() && mate_red.is_none() {
            return;
        }
        let Some(slot) = self.position_evaluations.get_mut(index) else {
            return;
        };
        if slot.is_some_and(|evaluation| evaluation.depth > depth) {
            return;
        }
        *slot = Some(PositionEvaluation {
            score_cp_red,
            mate_red,
            depth,
        });
    }

    fn schedule_next_search(&mut self) {
        if matches!(self.game.state(), PositionState::Won { .. }) {
            return;
        }
        let purpose = if self.human_mode.is_human_turn(self.game.side_to_move()) {
            SearchPurpose::Evaluation
        } else {
            SearchPurpose::AiMove
        };
        self.start_search(purpose, self.think_time);
    }

    fn start_search(&mut self, purpose: SearchPurpose, movetime: Duration) {
        if matches!(self.game.state(), PositionState::Won { .. }) {
            self.message = "棋局已经结束".to_string();
            return;
        }
        if self.thinking {
            self.invalidate_search();
        }
        self.search_sequence = self.search_sequence.wrapping_add(1);
        let id = self.search_sequence;
        let side = self.game.side_to_move();
        self.active_search = Some(ActiveSearch {
            id,
            purpose,
            side,
            position_index: self.game.history().len(),
            skill_move: None,
        });
        self.thinking = true;
        self.analysis_lines.clear();
        self.hint_move = None;
        match purpose {
            SearchPurpose::AiMove => self.message = format!("{}正在思考", color_name(side)),
            SearchPurpose::Hint => self.message = "正在分析局面".to_string(),
            SearchPurpose::Evaluation => {}
        }
        let multi_pv = match purpose {
            SearchPurpose::Hint => 3,
            SearchPurpose::AiMove if self.difficulty.is_full_strength() => 1,
            SearchPurpose::AiMove => 4,
            SearchPurpose::Evaluation => 1,
        };
        self.ai.search(SearchRequest {
            id,
            fen: self.game.fen(),
            movetime,
            purpose,
            multi_pv,
        });
    }

    fn cancel_search(&mut self, message: &str) {
        self.invalidate_search();
        self.message = message.to_string();
    }

    fn invalidate_search(&mut self) {
        self.search_sequence = self.search_sequence.wrapping_add(1);
        self.active_search = None;
        self.thinking = false;
        self.ai.stop();
    }

    fn poll_ai(&mut self) {
        while let Ok(event) = self.ai.try_recv() {
            self.handle_ai_event(event);
        }
    }

    fn handle_ai_event(&mut self, event: AiEvent) {
        match event {
            AiEvent::Ready { name, external } => {
                self.engine_name = name;
                self.external_engine = external;
                if !external {
                    self.message =
                        "正在使用内置轻量引擎；配置 Pikafish 后可获得更强棋力".to_string();
                }
                if self.active_search.is_none() {
                    self.schedule_next_search();
                }
            }
            AiEvent::Error(error) => self.message = error,
            AiEvent::Info { id, purpose, info } => {
                let Some(active) = self.active_search else {
                    return;
                };
                if active.id != id || active.purpose != purpose {
                    return;
                }
                let sign = if active.side == Color::Red { 1 } else { -1 };
                let moves = notation::parse_pv(&info.pv);
                let pv_text = notation::format_pv(self.game.position(), &moves, 12);
                let multipv = info.multipv.max(1);
                let depth = info.depth;
                let score_cp_red = info.score_cp.map(|score| score * sign);
                let mate_red = info.mate.map(|mate| mate * sign);
                if multipv == 1 {
                    self.update_position_evaluation(
                        active.position_index,
                        depth.unwrap_or(0),
                        score_cp_red,
                        mate_red,
                    );
                    if purpose != SearchPurpose::Evaluation
                        && let Some(first) = moves.first().copied()
                    {
                        self.hint_move = Some(first);
                    }
                }
                self.analysis_lines.insert(
                    multipv,
                    AnalysisLine {
                        score_cp_red,
                        mate_red,
                        info,
                        pv_text,
                    },
                );
                self.maybe_cache_skill_move(id, purpose, depth);
            }
            AiEvent::BestMove {
                id,
                purpose,
                best_move,
                ponder: _,
            } => {
                let Some(active) = self.active_search else {
                    return;
                };
                if active.id != id || active.purpose != purpose {
                    return;
                }
                self.active_search = None;
                self.thinking = false;
                if purpose == SearchPurpose::Evaluation {
                    return;
                }
                let Some(best_move) = best_move else {
                    self.message = "引擎没有找到可走着法".to_string();
                    return;
                };
                let Ok(engine_move) = Move::from_iccs(&best_move) else {
                    self.message = format!("引擎返回了无法识别的着法 {best_move}");
                    return;
                };
                let legal = self.game.legal_moves();
                if !legal.contains(&engine_move) {
                    self.message = format!("引擎返回了非法着法 {best_move}");
                    return;
                }
                let mv = if purpose == SearchPurpose::AiMove {
                    active
                        .skill_move
                        .filter(|mv| legal.contains(mv))
                        .or_else(|| self.pick_ai_candidate(&legal))
                        .unwrap_or(engine_move)
                } else {
                    engine_move
                };
                self.hint_move = Some(mv);
                match purpose {
                    SearchPurpose::Hint => {
                        self.message =
                            format!("建议 {}", notation::format_move(self.game.position(), mv));
                    }
                    SearchPurpose::AiMove => {
                        if let Ok(record) = self.game.play(mv) {
                            self.prepare_current_evaluation();
                            self.cursor = mv.dst();
                            self.clear_selection();
                            self.hint_move = None;
                            self.analysis_lines.clear();
                            self.after_move(format!("电脑走 {}", record.notation));
                        }
                    }
                    SearchPurpose::Evaluation => {}
                }
            }
        }
    }

    fn maybe_cache_skill_move(&mut self, id: u64, purpose: SearchPurpose, depth: Option<u32>) {
        // Pikafish freezes the weakened choice at search depth `level + 1`.
        let Some(target_depth) = self.difficulty.selection_depth() else {
            return;
        };
        let should_pick = purpose == SearchPurpose::AiMove
            && depth == Some(target_depth)
            && self
                .active_search
                .is_some_and(|active| active.id == id && active.skill_move.is_none())
            && self.analysis_lines.values().take(4).count() == 4
            && self
                .analysis_lines
                .values()
                .take(4)
                .all(|line| line.info.depth == Some(target_depth));
        if !should_pick {
            return;
        }

        let legal = self.game.legal_moves();
        let Some(mv) = self.pick_ai_candidate(&legal) else {
            return;
        };
        if let Some(active) = self.active_search.as_mut()
            && active.id == id
        {
            active.skill_move = Some(mv);
        }
    }

    fn pick_ai_candidate(&mut self, legal: &[Move]) -> Option<Move> {
        if self.difficulty.is_full_strength() {
            return None;
        }
        let candidates: Vec<(Move, i32)> = self
            .analysis_lines
            .values()
            .take(4)
            .filter_map(|line| {
                let mv = line
                    .info
                    .pv
                    .first()
                    .and_then(|value| Move::from_iccs(value).ok())?;
                if !legal.contains(&mv) {
                    return None;
                }
                Some((mv, analysis_score(&line.info)?))
            })
            .collect();
        if candidates.len() < 2 {
            return None;
        }
        let scores: Vec<i32> = candidates.iter().map(|(_, score)| *score).collect();
        let index = pick_skill_index(&scores, self.difficulty, |upper| {
            random_below(&mut self.random_state, upper)
        });
        Some(candidates[index].0)
    }

    fn previous_input(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => self.input_history.len() - 1,
            Some(index) => index.saturating_sub(1),
        };
        self.history_index = Some(index);
        self.input.clone_from(&self.input_history[index]);
    }

    fn next_input(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 >= self.input_history.len() {
            self.history_index = None;
            self.input.clear();
        } else {
            self.history_index = Some(index + 1);
            self.input.clone_from(&self.input_history[index + 1]);
        }
    }
}

fn difficulty_argument(command: &str) -> Option<&str> {
    command
        .strip_prefix("难度")
        .or_else(|| command.strip_prefix("/difficulty"))
        .or_else(|| command.strip_prefix("difficulty"))
}

fn difficulty_message(difficulty: Difficulty, current: bool) -> String {
    let prefix = if current {
        "当前 AI 难度"
    } else {
        "AI 难度已设为"
    };
    if difficulty.is_full_strength() {
        format!("{prefix} 20（不限棋力）")
    } else {
        format!("{prefix} {}", difficulty.level())
    }
}

fn analysis_score(info: &AnalysisInfo) -> Option<i32> {
    if let Some(mate) = info.mate {
        let distance = mate.unsigned_abs().min(1_000) as i32;
        return Some(if mate >= 0 {
            30_000 - distance
        } else {
            -30_000 + distance
        });
    }
    info.score_cp
}

fn pick_skill_index(
    scores: &[i32],
    difficulty: Difficulty,
    mut random_below: impl FnMut(u32) -> u32,
) -> usize {
    if scores.len() < 2 || difficulty.is_full_strength() {
        return 0;
    }

    // This is the same score weighting used by Pikafish's Skill Level picker.
    let top_score = i64::from(scores[0]);
    let last_score = i64::from(*scores.last().expect("at least two scores"));
    let delta = (top_score - last_score).clamp(0, 100);
    let weakness = i64::from(
        difficulty
            .weakness()
            .expect("full strength returned before weighted selection"),
    );
    let mut best_index = 0;
    let mut best_score = i64::MIN;

    for (index, score) in scores.iter().enumerate() {
        let score = i64::from(*score);
        let random = i64::from(random_below(weakness as u32) % weakness as u32);
        let push = (weakness * (top_score - score) + delta * random) / 128;
        let adjusted = score + push;
        if adjusted >= best_score {
            best_score = adjusted;
            best_index = index;
        }
    }
    best_index
}

fn random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    nanos as u64 ^ (nanos >> 64) as u64 ^ u64::from(std::process::id())
}

fn random_below(state: &mut u64, upper: u32) -> u32 {
    if upper <= 1 {
        return 0;
    }
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((value ^ (value >> 31)) as u32) % upper
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Instant;

    use super::*;

    #[test]
    fn human_modes_identify_turns() {
        assert!(HumanMode::Side(Color::Red).is_human_turn(Color::Red));
        assert!(!HumanMode::Side(Color::Red).is_human_turn(Color::Black));
        assert!(HumanMode::Both.is_human_turn(Color::Black));
    }

    #[test]
    fn chinese_command_then_ai_reply_completes_a_turn() {
        let mut app = App::new(AppOptions {
            engine_path: None,
            think_time: Duration::from_millis(50),
            difficulty: Difficulty::default(),
            emoji_pieces: true,
        });
        app.execute_command("马二进三");
        assert_eq!(app.game.history().len(), 1);
        assert!(app.thinking);

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.game.history().len() < 2 && Instant::now() < deadline {
            app.poll_ai();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.game.history().len(), 2);
        assert_eq!(app.game.side_to_move(), Color::Red);
    }

    #[test]
    fn background_evaluation_is_saved_from_reds_perspective() {
        let mut app = App::new(AppOptions::default());
        let mv = Move::from_iccs("h2e2").unwrap();
        app.game.play(mv).unwrap();
        app.prepare_current_evaluation();
        app.active_search = Some(ActiveSearch {
            id: 7,
            purpose: SearchPurpose::Evaluation,
            side: Color::Black,
            position_index: 1,
            skill_move: None,
        });
        app.thinking = true;

        app.handle_ai_event(AiEvent::Info {
            id: 7,
            purpose: SearchPurpose::Evaluation,
            info: AnalysisInfo {
                depth: Some(8),
                score_cp: Some(35),
                pv: vec!["h9g7".to_string()],
                ..AnalysisInfo::default()
            },
        });

        assert_eq!(
            app.position_evaluations[1],
            Some(PositionEvaluation {
                score_cp_red: Some(-35),
                mate_red: None,
                depth: 8,
            })
        );
        assert!(app.hint_move.is_none());
        assert!(!app.search_blocks_moves());
    }

    #[test]
    fn difficulty_accepts_only_levels_one_through_twenty() {
        assert_eq!(Difficulty::parse("1").unwrap().level(), 1);
        assert_eq!(Difficulty::parse("20").unwrap().level(), 20);
        assert!(Difficulty::parse("0").is_err());
        assert!(Difficulty::parse("21").is_err());
        assert!(Difficulty::parse("十").is_err());
    }

    #[test]
    fn difficulty_profiles_match_pikafish_skill_levels() {
        for level in 1..20 {
            let difficulty = Difficulty::new(level).unwrap();
            assert_eq!(difficulty.selection_depth(), Some(u32::from(level) + 1));
            assert_eq!(difficulty.weakness(), Some(120 - 2 * level));
        }
        assert_eq!(Difficulty::default().selection_depth(), None);
        assert_eq!(Difficulty::default().weakness(), None);
    }

    #[test]
    fn difficulty_command_sets_and_reports_the_level() {
        let mut app = App::new(AppOptions::default());
        app.execute_command("难度 7");
        assert_eq!(app.difficulty.level(), 7);
        assert_eq!(app.message, "AI 难度已设为 7");

        app.execute_command("难度");
        assert_eq!(app.message, "当前 AI 难度 7");

        app.execute_command("/difficulty 20");
        assert!(app.difficulty.is_full_strength());
        assert_eq!(app.message, "AI 难度已设为 20（不限棋力）");
    }

    #[test]
    fn stockfish_style_skill_picker_can_choose_a_weaker_line() {
        let rolls = [0, 0, 0, 117];
        let mut next = 0;
        let index = pick_skill_index(&[100, 90, 80, 70], Difficulty::new(1).unwrap(), |_| {
            let value = rolls[next];
            next += 1;
            value
        });
        assert_eq!(index, 3);
    }

    #[test]
    fn full_strength_always_keeps_the_engine_move() {
        let index = pick_skill_index(&[100, 90, 80, 70], Difficulty::default(), |_| {
            panic!("full strength must not use randomness")
        });
        assert_eq!(index, 0);
    }

    #[test]
    fn lower_difficulty_picks_at_its_target_depth() {
        let mut app = App::new(AppOptions {
            difficulty: Difficulty::new(1).unwrap(),
            ..AppOptions::default()
        });
        app.active_search = Some(ActiveSearch {
            id: 42,
            purpose: SearchPurpose::AiMove,
            side: Color::Red,
            position_index: 0,
            skill_move: None,
        });
        app.thinking = true;

        for (multipv, (score, pv)) in [(40, "h2e2"), (30, "c3c4"), (20, "g3g4"), (10, "b2e2")]
            .into_iter()
            .enumerate()
        {
            app.handle_ai_event(AiEvent::Info {
                id: 42,
                purpose: SearchPurpose::AiMove,
                info: AnalysisInfo {
                    depth: Some(2),
                    multipv: multipv as u32 + 1,
                    score_cp: Some(score),
                    pv: vec![pv.to_string()],
                    ..AnalysisInfo::default()
                },
            });
        }

        let skill_move = app.active_search.and_then(|search| search.skill_move);
        assert!(skill_move.is_some());
        assert!(app.game.legal_moves().contains(&skill_move.unwrap()));
    }
}
