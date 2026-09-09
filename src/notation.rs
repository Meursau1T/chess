use std::fmt;

use chessai::{Color, Move, Piece, PieceType, Position, Square};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveInputError {
    Empty,
    Illegal { suggestions: Vec<String> },
    Ambiguous { candidates: Vec<String> },
}

impl fmt::Display for MoveInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("请输入着法"),
            Self::Illegal { suggestions } if suggestions.is_empty() => {
                f.write_str("没有找到对应的合法着法")
            }
            Self::Illegal { suggestions } => {
                write!(f, "没有这步棋，可走：{}", suggestions.join("  "))
            }
            Self::Ambiguous { candidates } => {
                write!(f, "着法有歧义：{}", candidates.join("  "))
            }
        }
    }
}

impl std::error::Error for MoveInputError {}

#[derive(Clone)]
struct BoardSnapshot {
    cells: [Option<Piece>; 90],
    side: Color,
}

impl BoardSnapshot {
    fn from_position(position: &Position) -> Self {
        let mut cells = [None; 90];
        for rank in 0..10 {
            for file in 0..9 {
                let square = Square::from_rank_file(rank, file).expect("valid board square");
                cells[(rank as usize * 9) + file as usize] = position.piece_at(square);
            }
        }
        Self {
            cells,
            side: position.side_to_move(),
        }
    }

    fn piece_at(&self, square: Square) -> Option<Piece> {
        self.cells[(square.rank() as usize * 9) + square.file() as usize]
    }

    fn apply(&mut self, mv: Move) -> bool {
        let src = (mv.src().rank() as usize * 9) + mv.src().file() as usize;
        let dst = (mv.dst().rank() as usize * 9) + mv.dst().file() as usize;
        let Some(piece) = self.cells[src] else {
            return false;
        };
        self.cells[src] = None;
        self.cells[dst] = Some(piece);
        self.side = self.side.flip();
        true
    }
}

pub fn parse_move(
    input: &str,
    position: &Position,
    legal_moves: &[Move],
) -> Result<Move, MoveInputError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(MoveInputError::Empty);
    }

    let coordinate = trimmed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if let Ok(mv) = Move::from_iccs(&coordinate)
        && legal_moves.contains(&mv)
    {
        return Ok(mv);
    }

    let normalized = normalize_notation(trimmed);
    let mut matches = Vec::new();
    for &mv in legal_moves {
        let candidate = format_move(position, mv);
        if normalize_notation(&candidate) == normalized {
            matches.push(mv);
        }
    }

    match matches.as_slice() {
        [mv] => Ok(*mv),
        [] => Err(MoveInputError::Illegal {
            suggestions: suggestions_for(trimmed, position, legal_moves),
        }),
        _ => Err(MoveInputError::Ambiguous {
            candidates: matches
                .iter()
                .map(|&mv| format!("{} ({})", format_move(position, mv), compact_iccs(mv)))
                .collect(),
        }),
    }
}

pub fn format_move(position: &Position, mv: Move) -> String {
    format_move_on(&BoardSnapshot::from_position(position), mv)
}

pub fn format_pv(position: &Position, moves: &[Move], max_plies: usize) -> String {
    let mut board = BoardSnapshot::from_position(position);
    let mut result = Vec::new();
    for &mv in moves.iter().take(max_plies) {
        if board.piece_at(mv.src()).is_none() {
            break;
        }
        result.push(format_move_on(&board, mv));
        if !board.apply(mv) {
            break;
        }
    }
    result.join(" ")
}

pub fn parse_pv(raw: &[String]) -> Vec<Move> {
    raw.iter()
        .filter_map(|text| Move::from_iccs(text).ok())
        .collect()
}

fn format_move_on(board: &BoardSnapshot, mv: Move) -> String {
    let Some(piece) = board.piece_at(mv.src()) else {
        return compact_iccs(mv);
    };
    let side = piece.color();
    let name = piece_name(piece);
    let src = mv.src();
    let dst = mv.dst();

    let mut same_file = Vec::new();
    for rank in 0..10 {
        let square = Square::from_rank_file(rank, src.file()).expect("valid board square");
        if let Some(other) = board.piece_at(square)
            && other.color() == side
            && other.kind() == piece.kind()
        {
            same_file.push(square);
        }
    }
    same_file.sort_by(|a, b| match side {
        Color::Red => b.rank().cmp(&a.rank()),
        Color::Black => a.rank().cmp(&b.rank()),
    });

    let head = if same_file.len() <= 1 {
        format!(
            "{}{}",
            name,
            display_number(side, file_number(side, src.file()))
        )
    } else {
        let index = same_file.iter().position(|&sq| sq == src).unwrap_or(0);
        format!("{}{}", relative_prefix(index, same_file.len(), side), name)
    };

    if src.rank() == dst.rank() {
        return format!(
            "{}平{}",
            head,
            display_number(side, file_number(side, dst.file()))
        );
    }

    let forward = match side {
        Color::Red => dst.rank() > src.rank(),
        Color::Black => dst.rank() < src.rank(),
    };
    let action = if forward { "进" } else { "退" };
    let end_number = match piece.kind() {
        PieceType::Knight | PieceType::Bishop | PieceType::Advisor => file_number(side, dst.file()),
        PieceType::King | PieceType::Rook | PieceType::Cannon | PieceType::Pawn => {
            src.rank().abs_diff(dst.rank())
        }
    };
    format!("{}{}{}", head, action, display_number(side, end_number))
}

fn suggestions_for(input: &str, position: &Position, legal_moves: &[Move]) -> Vec<String> {
    let normalized = normalize_notation(input);
    let wanted_piece = normalized.chars().find(|c| "将士象马车炮兵".contains(*c));
    let mut candidates: Vec<String> = legal_moves
        .iter()
        .map(|&mv| format_move(position, mv))
        .filter(|text| wanted_piece.is_none_or(|piece| normalize_notation(text).contains(piece)))
        .collect();
    candidates.sort();
    candidates.dedup();
    candidates.truncate(6);
    candidates
}

fn relative_prefix(index: usize, count: usize, side: Color) -> String {
    match (index, count) {
        (0, _) => "前".to_string(),
        (i, n) if i + 1 == n => "后".to_string(),
        (1, 3) => "中".to_string(),
        (i, _) => display_number(side, (i + 1) as u8),
    }
}

fn piece_name(piece: Piece) -> &'static str {
    match (piece.color(), piece.kind()) {
        (Color::Red, PieceType::King) => "帅",
        (Color::Black, PieceType::King) => "将",
        (Color::Red, PieceType::Advisor) => "仕",
        (Color::Black, PieceType::Advisor) => "士",
        (Color::Red, PieceType::Bishop) => "相",
        (Color::Black, PieceType::Bishop) => "象",
        (_, PieceType::Knight) => "马",
        (_, PieceType::Rook) => "车",
        (_, PieceType::Cannon) => "炮",
        (Color::Red, PieceType::Pawn) => "兵",
        (Color::Black, PieceType::Pawn) => "卒",
    }
}

fn file_number(side: Color, file: u8) -> u8 {
    match side {
        Color::Red => 9 - file,
        Color::Black => file + 1,
    }
}

fn display_number(side: Color, number: u8) -> String {
    if side == Color::Black {
        return number.to_string();
    }
    match number {
        1 => "一",
        2 => "二",
        3 => "三",
        4 => "四",
        5 => "五",
        6 => "六",
        7 => "七",
        8 => "八",
        9 => "九",
        _ => "?",
    }
    .to_string()
}

fn compact_iccs(mv: Move) -> String {
    format!("{}{}", mv.src(), mv.dst())
}

pub fn normalize_notation(input: &str) -> String {
    input
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() || matches!(c, '-' | '－' | '—' | '_') {
                return None;
            }
            Some(match c {
                '１' | '一' | '壹' => '1',
                '２' | '二' | '贰' | '貳' | '两' | '兩' => '2',
                '３' | '三' | '叁' | '參' => '3',
                '４' | '四' | '肆' => '4',
                '５' | '五' | '伍' => '5',
                '６' | '六' | '陆' | '陸' => '6',
                '７' | '七' | '柒' => '7',
                '８' | '八' | '捌' => '8',
                '９' | '九' | '玖' => '9',
                '車' | '俥' => '车',
                '馬' | '傌' => '马',
                '帥' | '帅' | '將' => '将',
                '仕' => '士',
                '相' => '象',
                '砲' => '炮',
                '卒' => '兵',
                '進' => '进',
                '後' => '后',
                other => other.to_ascii_lowercase(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chessai::Engine;

    use super::*;

    fn start() -> Engine {
        Engine::builder()
            .hash_size(1)
            .threads(1)
            .use_book(false)
            .build()
    }

    #[test]
    fn formats_common_red_opening_moves() {
        let engine = start();
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("h0g2").unwrap()),
            "马二进三"
        );
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("h2e2").unwrap()),
            "炮二平五"
        );
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("a0a1").unwrap()),
            "车九进一"
        );
    }

    #[test]
    fn formats_black_from_black_perspective() {
        let mut engine = start();
        engine
            .set_fen("rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR b - - 0 1")
            .unwrap();
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("h9g7").unwrap()),
            "马8进7"
        );
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("b9c7").unwrap()),
            "马2进3"
        );
    }

    #[test]
    fn parses_simplified_traditional_and_coordinates() {
        let mut engine = start();
        let legal = engine.legal_moves();
        assert_eq!(
            parse_move("马二进三", engine.position(), &legal)
                .unwrap()
                .to_iccs(),
            "h0-g2"
        );
        assert_eq!(
            parse_move("馬二進三", engine.position(), &legal)
                .unwrap()
                .to_iccs(),
            "h0-g2"
        );
        assert_eq!(
            parse_move("h0g2", engine.position(), &legal)
                .unwrap()
                .to_iccs(),
            "h0-g2"
        );
    }

    #[test]
    fn front_and_rear_pieces_are_disambiguated() {
        let mut engine = start();
        engine
            .set_fen("4k4/9/9/9/9/9/R8/9/R8/4K4 w - - 0 1")
            .unwrap();
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("a3a4").unwrap()),
            "前车进一"
        );
        assert_eq!(
            format_move(engine.position(), Move::from_iccs("a1a2").unwrap()),
            "后车进一"
        );
    }

    #[test]
    fn normalization_accepts_piece_aliases() {
        assert_eq!(normalize_notation(" 俥九進一 "), "车9进1");
        assert_eq!(normalize_notation("砲２平５"), "炮2平5");
    }
}
