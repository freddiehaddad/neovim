use std::fmt;

/// Represents different editor modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    VisualBlock,
    Command,
    Replace,
    Search,
    OperatorPending, // For waiting for text object after operator
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Normal => write!(f, "NORMAL"),
            Mode::Insert => write!(f, "INSERT"),
            Mode::Visual => write!(f, "VISUAL"),
            Mode::VisualLine => write!(f, "V-LINE"),
            Mode::VisualBlock => write!(f, "V-BLOCK"),
            Mode::Command => write!(f, "COMMAND"),
            Mode::Replace => write!(f, "REPLACE"),
            Mode::Search => write!(f, "SEARCH"),
            Mode::OperatorPending => write!(f, "OP-PENDING"),
        }
    }
}

/// Cursor position in the buffer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

impl Position {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }

    pub fn zero() -> Self {
        Self { row: 0, col: 0 }
    }
}

/// Selection range for visual mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

impl Selection {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}
