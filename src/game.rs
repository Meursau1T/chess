use chessai::{Color, Engine, Move, Piece, Position, Square};

use crate::notation;

#[derive(Debug, Clone)]
pub struct MoveRecord {
    pub mv: Move,
    pub notation: String,
    pub side: Color,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionState {
    Playing,
    Check(Color),
    Won { winner: Color, by_checkmate: bool },
}

pub struct Game {
    rules: Engine,
    records: Vec<MoveRecord>,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        Self {
            rules: Engine::builder()
                .hash_size(1)
                .threads(1)
                .use_book(false)
                .build(),
            records: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.rules.reset_to_startpos();
        self.records.clear();
    }

    pub fn set_fen(&mut self, fen: &str) -> anyhow::Result<()> {
        self.rules.set_fen(fen)?;
        self.records.clear();
        Ok(())
    }

    pub fn fen(&self) -> String {
        self.rules.fen()
    }

    pub fn side_to_move(&self) -> Color {
        self.rules.side_to_move()
    }

    pub fn position(&self) -> &Position {
        self.rules.position()
    }

    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        self.rules.position().piece_at(square)
    }

    pub fn legal_moves(&mut self) -> Vec<Move> {
        self.rules.legal_moves()
    }

    pub fn legal_moves_from(&mut self, square: Square) -> Vec<Move> {
        self.rules
            .legal_moves()
            .into_iter()
            .filter(|mv| mv.src() == square)
            .collect()
    }

    pub fn play(&mut self, mv: Move) -> anyhow::Result<MoveRecord> {
        let side = self.side_to_move();
        let notation = notation::format_move(self.position(), mv);
        if !self.rules.make_move(mv) {
            anyhow::bail!("这步棋不合法");
        }
        let record = MoveRecord { mv, notation, side };
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn undo(&mut self) -> Option<MoveRecord> {
        self.rules.undo_move()?;
        self.records.pop()
    }

    pub fn history(&self) -> &[MoveRecord] {
        &self.records
    }

    pub fn last_move(&self) -> Option<Move> {
        self.records.last().map(|record| record.mv)
    }

    pub fn state(&mut self) -> PositionState {
        let side = self.side_to_move();
        if self.position().king_square(side).is_none() {
            return PositionState::Won {
                winner: side.flip(),
                by_checkmate: true,
            };
        }

        let in_check = self.position().is_in_check(side);
        if self.rules.legal_moves().is_empty() {
            return PositionState::Won {
                winner: side.flip(),
                by_checkmate: in_check,
            };
        }

        if in_check {
            PositionState::Check(side)
        } else {
            PositionState::Playing
        }
    }
}

pub fn color_name(color: Color) -> &'static str {
    match color {
        Color::Red => "红方",
        Color::Black => "黑方",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_position_has_expected_moves() {
        let mut game = Game::new();
        let legal = game.legal_moves();
        assert!(legal.iter().any(|mv| mv.to_iccs() == "h0-g2"));
        assert!(legal.iter().any(|mv| mv.to_iccs() == "h2-e2"));
    }

    #[test]
    fn play_and_undo_roundtrip() {
        let mut game = Game::new();
        let before = game.fen();
        let mv = Move::from_iccs("h0g2").unwrap();
        game.play(mv).unwrap();
        assert_eq!(game.side_to_move(), Color::Black);
        game.undo().unwrap();
        assert_eq!(game.fen(), before);
    }
}
