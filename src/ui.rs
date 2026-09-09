use chessai::{Color as SideColor, Piece, PieceType, Square};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{AnalysisLine, App, Focus, PositionEvaluation};
use crate::game::{PositionState, color_name};

const BOARD_WIDTH: u16 = 42;
const BOARD_HEIGHT: u16 = 14;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 39 || area.height < 19 {
        frame.render_widget(
            Paragraph::new("终端窗口太小，请至少调整到 39×19")
                .block(Block::default().borders(Borders::ALL).title("象棋"))
                .style(Style::default().fg(Color::Yellow)),
            area,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(BOARD_HEIGHT),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_status(frame, rows[0], app);

    if rows[1].width >= 78 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(BOARD_WIDTH), Constraint::Min(32)])
            .split(rows[1]);
        frame.render_widget(BoardWidget { app }, columns[0]);
        render_sidebar(frame, columns[1], app);
    } else {
        frame.render_widget(BoardWidget { app }, rows[1]);
    }

    render_input(frame, rows[2], app);
    frame.render_widget(
        Paragraph::new(app.message.as_str()).style(Style::default().fg(Color::Yellow)),
        rows[3],
    );
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let side = app.game.side_to_move();
    let thinking = if app.is_evaluating() {
        "  ◉ 评估中"
    } else if app.thinking {
        "  ◉ 思考中"
    } else {
        ""
    };
    let backend = if app.external_engine {
        "Pikafish"
    } else {
        "轻量引擎"
    };
    let line = Line::from(vec![
        Span::styled(
            " 象棋 ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {}行棋  {}  {}  难度 {}",
            color_name(side),
            app.human_mode.label(),
            backend,
            app.difficulty.level()
        )),
        Span::styled(thinking, Style::default().fg(Color::LightCyan)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_sidebar(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(5)])
        .split(area);
    render_analysis(frame, sections[0], app);
    render_history(frame, sections[1], app);
}

fn render_analysis(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let border_style = if app.thinking {
        Style::default().fg(Color::LightCyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("引擎  ", Style::default().fg(Color::DarkGray)),
            Span::raw(app.engine_name.clone()),
        ]),
        position_score_line(app),
    ];

    if let Some(primary) = app.analysis_lines.get(&1) {
        lines.push(stat_line(primary));
    } else {
        lines.push(Line::styled(
            "等待分析",
            Style::default().fg(Color::DarkGray),
        ));
    }

    for (index, line) in app.analysis_lines.iter().take(3) {
        let score = format_score(line);
        let text = if line.pv_text.is_empty() {
            "尚无变化".to_string()
        } else {
            line.pv_text.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{index} {score:>7}  "),
                Style::default().fg(score_color(line)),
            ),
            Span::raw(text),
        ]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" AI 分析 ")
                    .border_style(border_style),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn position_score_line(app: &App) -> Line<'static> {
    let index = app.game.history().len();
    let current = app.position_evaluations.get(index).and_then(Option::as_ref);
    let previous = index
        .checked_sub(1)
        .and_then(|previous| app.position_evaluations.get(previous))
        .and_then(Option::as_ref);

    let Some(current) = current else {
        let state = if app.is_evaluating() {
            "计算中"
        } else {
            "尚无评分"
        };
        return Line::from(vec![
            Span::styled("局势  ", Style::default().fg(Color::DarkGray)),
            Span::styled(state, Style::default().fg(Color::DarkGray)),
        ]);
    };

    let score = format_evaluation(current.score_cp_red, current.mate_red);
    let advantage = evaluation_advantage(current);
    let mut spans = vec![
        Span::styled("局势  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{advantage} {score}"),
            Style::default().fg(evaluation_color(current)),
        ),
    ];
    if let Some(delta) = evaluation_delta(current, previous) {
        spans.push(Span::styled(
            format!("  变化 {:+.2}", delta as f64 / 100.0),
            Style::default().fg(if delta > 0 {
                Color::LightRed
            } else if delta < 0 {
                Color::LightBlue
            } else {
                Color::Gray
            }),
        ));
    }
    Line::from(spans)
}

fn stat_line(line: &AnalysisLine) -> Line<'static> {
    let depth = line.info.depth.unwrap_or(0);
    let time = line.info.time_ms.unwrap_or(0);
    let nps = format_count(line.info.nps.unwrap_or(0));
    Line::from(vec![
        Span::styled("深度 ", Style::default().fg(Color::DarkGray)),
        Span::raw(depth.to_string()),
        Span::styled("  用时 ", Style::default().fg(Color::DarkGray)),
        Span::raw(format!("{:.1}s", time as f64 / 1000.0)),
        Span::styled("  NPS ", Style::default().fg(Color::DarkGray)),
        Span::raw(nps),
    ])
}

fn format_score(line: &AnalysisLine) -> String {
    format_evaluation(line.score_cp_red, line.mate_red)
}

fn format_evaluation(score_cp_red: Option<i32>, mate_red: Option<i32>) -> String {
    if let Some(mate) = mate_red {
        if mate > 0 {
            return format!("红杀{mate}");
        }
        return format!("黑杀{}", mate.unsigned_abs());
    }
    score_cp_red
        .map(|score| format!("{:+.2}", score as f64 / 100.0))
        .unwrap_or_else(|| "--".to_string())
}

fn score_color(line: &AnalysisLine) -> Color {
    evaluation_values_color(line.score_cp_red, line.mate_red)
}

fn evaluation_color(evaluation: &PositionEvaluation) -> Color {
    evaluation_values_color(evaluation.score_cp_red, evaluation.mate_red)
}

fn evaluation_values_color(score_cp_red: Option<i32>, mate_red: Option<i32>) -> Color {
    if mate_red.unwrap_or(0) > 0 || score_cp_red.unwrap_or(0) > 20 {
        Color::LightRed
    } else if mate_red.unwrap_or(0) < 0 || score_cp_red.unwrap_or(0) < -20 {
        Color::LightBlue
    } else {
        Color::Gray
    }
}

fn evaluation_advantage(evaluation: &PositionEvaluation) -> &'static str {
    if evaluation.mate_red.unwrap_or(0) > 0 {
        "红胜"
    } else if evaluation.mate_red.unwrap_or(0) < 0 {
        "黑胜"
    } else if evaluation.score_cp_red.unwrap_or(0) > 20 {
        "红优"
    } else if evaluation.score_cp_red.unwrap_or(0) < -20 {
        "黑优"
    } else {
        "均势"
    }
}

fn evaluation_delta(
    current: &PositionEvaluation,
    previous: Option<&PositionEvaluation>,
) -> Option<i32> {
    let previous = previous?;
    Some(current.score_cp_red? - previous.score_cp_red?)
}

fn render_history(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (ply, record) in app.game.history().iter().enumerate() {
        let evaluation = app
            .position_evaluations
            .get(ply + 1)
            .and_then(Option::as_ref);
        let previous = app.position_evaluations.get(ply).and_then(Option::as_ref);
        let score = evaluation
            .map(|value| format_evaluation(value.score_cp_red, value.mate_red))
            .unwrap_or_else(|| "--".to_string());
        let delta = evaluation
            .and_then(|value| evaluation_delta(value, previous))
            .map(|value| format!("  Δ{:+.2}", value as f64 / 100.0))
            .unwrap_or_default();
        let side_label = if record.side == SideColor::Red {
            "红"
        } else {
            "黑"
        };
        let side_color = if record.side == SideColor::Red {
            Color::LightRed
        } else {
            Color::LightBlue
        };
        let score_color = evaluation.map(evaluation_color).unwrap_or(Color::DarkGray);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>3}.{side_label} ", ply / 2 + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                pad_to_width(&record.notation, 10),
                Style::default().fg(side_color),
            ),
            Span::styled(score, Style::default().fg(score_color)),
            Span::styled(delta, Style::default().fg(score_color)),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::styled(
            "尚未走棋",
            Style::default().fg(Color::DarkGray),
        ));
    }
    let visible = area.height.saturating_sub(2) as usize;
    let scroll = lines.len().saturating_sub(visible) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 棋谱与评分 ")
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .scroll((scroll, 0)),
        area,
    );
}

fn render_input(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let focused = app.focus == Focus::Input;
    let border = if focused {
        Color::LightGreen
    } else {
        Color::DarkGray
    };
    let prompt = if focused { "❯ " } else { "  " };
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(prompt, Style::default().fg(Color::LightGreen)),
        Span::raw(app.input.as_str()),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 指令 ")
            .border_style(Style::default().fg(border)),
    );
    frame.render_widget(paragraph, area);

    if focused {
        let input_width = UnicodeWidthStr::width(app.input.as_str()) as u16;
        let cursor_x = area
            .x
            .saturating_add(3)
            .saturating_add(input_width)
            .min(area.right().saturating_sub(2));
        frame.set_cursor_position((cursor_x, area.y.saturating_add(1)));
    }
}

struct BoardWidget<'a> {
    app: &'a App,
}

impl Widget for BoardWidget<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let border_color = if self.app.focus == Focus::Board {
            Color::LightGreen
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" 棋盘 · Tab 切换焦点 ")
            .border_style(Style::default().fg(border_color));
        let inner = block.inner(area);
        block.render(area, buffer);
        if inner.height < 12 || inner.width < 37 {
            buffer.set_string(
                inner.x,
                inner.y,
                "棋盘区域太小",
                Style::default().fg(Color::Yellow),
            );
            return;
        }

        let content_width = 38u16;
        let origin_x = inner.x + inner.width.saturating_sub(content_width) / 2;
        let origin_y = inner.y + inner.height.saturating_sub(12) / 2;

        for display_column in 0..9u8 {
            let x = origin_x + 3 + display_column as u16 * 4;
            buffer.set_string(
                x,
                origin_y,
                (display_column + 1).to_string(),
                Style::default().fg(Color::DarkGray),
            );
        }

        for display_row in 0..10u8 {
            let rank = if self.app.flipped {
                display_row
            } else {
                9 - display_row
            };
            let y = origin_y + 1 + display_row as u16;
            buffer.set_string(
                origin_x,
                y,
                rank.to_string(),
                Style::default().fg(Color::DarkGray),
            );
            for display_column in 0..9u8 {
                let file = if self.app.flipped {
                    8 - display_column
                } else {
                    display_column
                };
                let square = Square::from_rank_file(rank, file).expect("valid board square");
                let x = origin_x + 3 + display_column as u16 * 4;
                render_square(buffer, x, y, square, self.app);
            }
        }

        for display_column in 0..9u8 {
            let x = origin_x + 3 + display_column as u16 * 4;
            buffer.set_string(
                x,
                origin_y + 11,
                (9 - display_column).to_string(),
                Style::default().fg(Color::DarkGray),
            );
        }
    }
}

fn render_square(buffer: &mut Buffer, x: u16, y: u16, square: Square, app: &App) {
    let piece = app.game.piece_at(square);
    let is_legal_target = app.selected_moves.iter().any(|mv| mv.dst() == square);
    let mut style = match piece.map(|piece| piece.color()) {
        Some(SideColor::Red) => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        Some(SideColor::Black) => Style::default()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::BOLD),
        None if is_legal_target => Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
        None => Style::default().fg(Color::DarkGray),
    };

    if let Some(last) = app.game.last_move()
        && (last.src() == square || last.dst() == square)
    {
        style = style.bg(Color::Rgb(70, 58, 22));
    }
    if let Some(hint) = app.hint_move
        && (hint.src() == square || hint.dst() == square)
    {
        style = style.bg(Color::Rgb(64, 34, 76));
    }
    if is_legal_target && piece.is_some() {
        style = style.bg(Color::Cyan).fg(Color::Black);
    }
    if app.selected == Some(square) {
        style = style.bg(Color::Green).fg(Color::Black);
    }
    if app.focus == Focus::Board && app.cursor == square {
        style = style.add_modifier(Modifier::REVERSED);
    }

    let symbol = piece
        .map(|piece| piece_symbol(piece, app.emoji_pieces))
        .unwrap_or("·");
    buffer.set_string(x, y, symbol, style);
}

pub fn piece_symbol(piece: Piece, emoji: bool) -> &'static str {
    if emoji {
        return match (piece.color(), piece.kind()) {
            (SideColor::Red, PieceType::King) => "🩠",
            (SideColor::Red, PieceType::Advisor) => "🩡",
            (SideColor::Red, PieceType::Bishop) => "🩢",
            (SideColor::Red, PieceType::Knight) => "🩣",
            (SideColor::Red, PieceType::Rook) => "🩤",
            (SideColor::Red, PieceType::Cannon) => "🩥",
            (SideColor::Red, PieceType::Pawn) => "🩦",
            (SideColor::Black, PieceType::King) => "🩧",
            (SideColor::Black, PieceType::Advisor) => "🩨",
            (SideColor::Black, PieceType::Bishop) => "🩩",
            (SideColor::Black, PieceType::Knight) => "🩪",
            (SideColor::Black, PieceType::Rook) => "🩫",
            (SideColor::Black, PieceType::Cannon) => "🩬",
            (SideColor::Black, PieceType::Pawn) => "🩭",
        };
    }
    match (piece.color(), piece.kind()) {
        (SideColor::Red, PieceType::King) => "帅",
        (SideColor::Red, PieceType::Advisor) => "仕",
        (SideColor::Red, PieceType::Bishop) => "相",
        (SideColor::Red, PieceType::Knight) => "马",
        (SideColor::Red, PieceType::Rook) => "车",
        (SideColor::Red, PieceType::Cannon) => "炮",
        (SideColor::Red, PieceType::Pawn) => "兵",
        (SideColor::Black, PieceType::King) => "将",
        (SideColor::Black, PieceType::Advisor) => "士",
        (SideColor::Black, PieceType::Bishop) => "象",
        (SideColor::Black, PieceType::Knight) => "马",
        (SideColor::Black, PieceType::Rook) => "车",
        (SideColor::Black, PieceType::Cannon) => "炮",
        (SideColor::Black, PieceType::Pawn) => "卒",
    }
}

fn format_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn pad_to_width(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

#[allow(dead_code)]
fn state_label(state: &PositionState) -> String {
    match state {
        PositionState::Playing => "对局中".to_string(),
        PositionState::Check(side) => format!("{}被将军", color_name(*side)),
        PositionState::Won { winner, .. } => format!("{}获胜", color_name(*winner)),
    }
}

#[cfg(test)]
mod tests {
    use chessai::{Color as SideColor, Piece, PieceType};
    use ratatui::{Terminal, backend::TestBackend};

    use crate::app::{App, AppOptions};

    use super::*;

    #[test]
    fn uses_unicode_xiangqi_symbols() {
        assert_eq!(
            piece_symbol(Piece::new(SideColor::Red, PieceType::King), true),
            "🩠"
        );
        assert_eq!(
            piece_symbol(Piece::new(SideColor::Black, PieceType::Pawn), true),
            "🩭"
        );
    }

    #[test]
    fn full_layout_contains_text_pieces_dots_and_analysis_panel() {
        let app = App::new(AppOptions::default());
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let symbols = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(symbols.contains('帅'));
        assert!(symbols.contains('·'));
        assert!(symbols.contains("AI"));
    }

    #[test]
    fn history_shows_position_score_and_change() {
        let mut app = App::new(AppOptions::default());
        app.game
            .play(chessai::Move::from_iccs("h2e2").unwrap())
            .unwrap();
        app.position_evaluations = vec![
            Some(PositionEvaluation {
                score_cp_red: Some(10),
                mate_red: None,
                depth: 8,
            }),
            Some(PositionEvaluation {
                score_cp_red: Some(35),
                mate_red: None,
                depth: 8,
            }),
        ];
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let symbols = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(symbols.contains("+0.35"));
        assert!(symbols.contains("Δ+0.25"));
    }

    #[test]
    fn board_uses_opposite_numeric_file_labels() {
        let app = App::new(AppOptions::default());
        let backend = TestBackend::new(42, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| frame.render_widget(BoardWidget { app: &app }, frame.area()))
            .unwrap();
        let cells = terminal.backend().buffer().content();
        let row_digits = |row: usize| {
            cells[row * 42..(row + 1) * 42]
                .iter()
                .flat_map(|cell| cell.symbol().chars())
                .filter(char::is_ascii_digit)
                .collect::<String>()
        };
        assert_eq!(row_digits(1), "123456789");
        assert_eq!(row_digits(12), "987654321");
    }
}
