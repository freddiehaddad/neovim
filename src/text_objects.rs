/// Text object system for Vim-style operations
/// Implements text objects like 'iw' (inner word), 'aw' (a word), 'ip' (inner paragraph), etc.
use crate::buffer::Buffer;
use crate::mode::Position;
use anyhow::Result;
use log::{debug, trace};

#[cfg(test)]
mod tests;

/// Represents the type of text object
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectType {
    Word,      // w - word
    Word2,     // W - WORD (whitespace delimited)
    Sentence,  // s - sentence
    Paragraph, // p - paragraph
    Quote,     // " or ' - quoted text
    Paren,     // () - parentheses
    Bracket,   // [] - square brackets
    Brace,     // {} - curly braces
    Angle,     // <> - angle brackets
    Tag,       // t - HTML/XML tag
}

/// Represents whether we want inner or around (a) text object
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectMode {
    Inner,  // i - inner (exclude delimiters)
    Around, // a - around (include delimiters)
}

/// A range of text selected by a text object
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextObjectRange {
    pub start: Position,
    pub end: Position,
    pub object_type: TextObjectType,
    pub mode: TextObjectMode,
}

impl TextObjectRange {
    pub fn new(
        start: Position,
        end: Position,
        object_type: TextObjectType,
        mode: TextObjectMode,
    ) -> Self {
        Self {
            start,
            end,
            object_type,
            mode,
        }
    }

    /// Check if this range is valid (start comes before or equals end)
    pub fn is_valid(&self) -> bool {
        self.start.row < self.end.row
            || (self.start.row == self.end.row && self.start.col <= self.end.col)
    }

    /// Get the text content for this range from the buffer
    pub fn get_text(&self, buffer: &Buffer) -> String {
        if !self.is_valid() {
            return String::new();
        }

        if self.start.row == self.end.row {
            // Single line selection
            if let Some(line) = buffer.lines.get(self.start.row) {
                let start_col = self.start.col.min(line.len());
                let end_col = self.end.col.min(line.len());
                return line[start_col..end_col].to_string();
            }
        } else {
            // Multi-line selection
            let mut result = String::new();

            // First line (from start_col to end)
            if let Some(line) = buffer.lines.get(self.start.row) {
                let start_col = self.start.col.min(line.len());
                result.push_str(&line[start_col..]);
                result.push('\n');
            }

            // Middle lines (complete lines)
            for row in (self.start.row + 1)..self.end.row {
                if let Some(line) = buffer.lines.get(row) {
                    result.push_str(line);
                    result.push('\n');
                }
            }

            // Last line (from start to end_col)
            if let Some(line) = buffer.lines.get(self.end.row) {
                let end_col = self.end.col.min(line.len());
                result.push_str(&line[..end_col]);
            }

            return result;
        }

        String::new()
    }
}

/// Text object finder - finds text object ranges at cursor position
pub struct TextObjectFinder {
    /// Characters that separate words
    pub word_separators: String,
    /// Characters that end sentences
    pub sentence_enders: String,
    /// Matching pairs for brackets/quotes
    pub bracket_pairs: Vec<(char, char)>,
}

impl Default for TextObjectFinder {
    fn default() -> Self {
        Self {
            word_separators: " \t\n\r.,;:!?()[]{}\"'<>".to_string(),
            sentence_enders: ".!?".to_string(),
            bracket_pairs: vec![
                ('(', ')'),
                ('[', ']'),
                ('{', '}'),
                ('<', '>'),
                ('"', '"'),
                ('\'', '\''),
                ('`', '`'),
            ],
        }
    }
}

impl TextObjectFinder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Find a text object at the given cursor position
    pub fn find_text_object(
        &self,
        buffer: &Buffer,
        cursor: Position,
        object_type: TextObjectType,
        mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        trace!(
            "Finding text object {:?} {:?} at position {:?}",
            mode, object_type, cursor
        );

        match object_type {
            TextObjectType::Word => self.find_word(buffer, cursor, mode),
            TextObjectType::Word2 => self.find_word2(buffer, cursor, mode),
            TextObjectType::Sentence => self.find_sentence(buffer, cursor, mode),
            TextObjectType::Paragraph => self.find_paragraph(buffer, cursor, mode),
            TextObjectType::Quote => self.find_quote(buffer, cursor, mode),
            TextObjectType::Paren => self.find_bracket_pair(buffer, cursor, mode, '(', ')'),
            TextObjectType::Bracket => self.find_bracket_pair(buffer, cursor, mode, '[', ']'),
            TextObjectType::Brace => self.find_bracket_pair(buffer, cursor, mode, '{', '}'),
            TextObjectType::Angle => self.find_bracket_pair(buffer, cursor, mode, '<', '>'),
            TextObjectType::Tag => self.find_tag(buffer, cursor, mode),
        }
    }

    /// Find word text object (separated by whitespace and punctuation)
    fn find_word(
        &self,
        buffer: &Buffer,
        cursor: Position,
        mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        let line = match buffer.lines.get(cursor.row) {
            Some(line) => line,
            None => return Ok(None),
        };

        if line.is_empty() {
            return Ok(None);
        }

        let chars: Vec<char> = line.chars().collect();
        let cursor_col = cursor.col.min(chars.len().saturating_sub(1));

        // If we're on a separator, handle differently based on mode
        if cursor_col < chars.len() && self.is_word_separator(chars[cursor_col]) {
            return match mode {
                TextObjectMode::Inner => Ok(None), // No inner object on separator
                TextObjectMode::Around => self.find_whitespace_around(buffer, cursor),
            };
        }

        // Find word boundaries
        let word_start = self.find_word_start(&chars, cursor_col);
        let word_end = self.find_word_end(&chars, cursor_col);

        let mut start_pos = Position::new(cursor.row, word_start);
        let mut end_pos = Position::new(cursor.row, word_end);

        // For "around" mode, include surrounding whitespace
        if mode == TextObjectMode::Around {
            // Try to include trailing whitespace first
            let mut extended_end = word_end;
            while extended_end < chars.len() && chars[extended_end].is_whitespace() {
                extended_end += 1;
            }

            // If no trailing whitespace, try leading whitespace
            if extended_end == word_end && word_start > 0 {
                let mut extended_start = word_start;
                while extended_start > 0 && chars[extended_start - 1].is_whitespace() {
                    extended_start -= 1;
                }
                start_pos.col = extended_start;
            } else {
                end_pos.col = extended_end;
            }
        }

        debug!(
            "Found word object: {:?} from {:?} to {:?}",
            mode, start_pos, end_pos
        );

        Ok(Some(TextObjectRange::new(
            start_pos,
            end_pos,
            TextObjectType::Word,
            mode,
        )))
    }

    /// Find WORD text object (separated only by whitespace)
    fn find_word2(
        &self,
        buffer: &Buffer,
        cursor: Position,
        mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        let line = match buffer.lines.get(cursor.row) {
            Some(line) => line,
            None => return Ok(None),
        };

        if line.is_empty() {
            return Ok(None);
        }

        let chars: Vec<char> = line.chars().collect();
        let cursor_col = cursor.col.min(chars.len().saturating_sub(1));

        // If we're on whitespace, handle based on mode
        if cursor_col < chars.len() && chars[cursor_col].is_whitespace() {
            return match mode {
                TextObjectMode::Inner => Ok(None),
                TextObjectMode::Around => self.find_whitespace_around(buffer, cursor),
            };
        }

        // Find WORD boundaries (only whitespace separates)
        let word_start = self.find_word2_start(&chars, cursor_col);
        let word_end = self.find_word2_end(&chars, cursor_col);

        let mut start_pos = Position::new(cursor.row, word_start);
        let mut end_pos = Position::new(cursor.row, word_end);

        // For "around" mode, include surrounding whitespace
        if mode == TextObjectMode::Around {
            // Try to include trailing whitespace
            let mut extended_end = word_end;
            while extended_end < chars.len() && chars[extended_end].is_whitespace() {
                extended_end += 1;
            }

            if extended_end == word_end && word_start > 0 {
                let mut extended_start = word_start;
                while extended_start > 0 && chars[extended_start - 1].is_whitespace() {
                    extended_start -= 1;
                }
                start_pos.col = extended_start;
            } else {
                end_pos.col = extended_end;
            }
        }

        Ok(Some(TextObjectRange::new(
            start_pos,
            end_pos,
            TextObjectType::Word2,
            mode,
        )))
    }

    /// Find sentence text object
    fn find_sentence(
        &self,
        buffer: &Buffer,
        cursor: Position,
        mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        // For now, implement a simple sentence finder
        // This could be enhanced to handle more complex sentence detection
        let _current_line = cursor.row;

        // Find sentence start (previous sentence end + 1 or beginning of paragraph)
        let sentence_start = self.find_sentence_start(buffer, cursor)?;

        // Find sentence end (next sentence delimiter)
        let sentence_end = self.find_sentence_end(buffer, cursor)?;

        if let (Some(start), Some(end)) = (sentence_start, sentence_end) {
            let start_pos = start;
            let mut end_pos = end;

            if mode == TextObjectMode::Around {
                // Include trailing whitespace for "around" mode
                if let Some(line) = buffer.lines.get(end_pos.row) {
                    let chars: Vec<char> = line.chars().collect();
                    let mut extended_end = end_pos.col;
                    while extended_end < chars.len() && chars[extended_end].is_whitespace() {
                        extended_end += 1;
                    }
                    end_pos.col = extended_end;
                }
            }

            return Ok(Some(TextObjectRange::new(
                start_pos,
                end_pos,
                TextObjectType::Sentence,
                mode,
            )));
        }

        Ok(None)
    }

    /// Find paragraph text object
    fn find_paragraph(
        &self,
        buffer: &Buffer,
        cursor: Position,
        mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        let current_row = cursor.row;

        // Find paragraph start (first non-empty line going backward)
        let mut start_row = current_row;
        while start_row > 0 {
            if let Some(line) = buffer.lines.get(start_row - 1) {
                if line.trim().is_empty() {
                    break;
                }
                start_row -= 1;
            } else {
                break;
            }
        }

        // Find paragraph end (first empty line going forward)
        let mut end_row = current_row;
        while end_row < buffer.lines.len() {
            if let Some(line) = buffer.lines.get(end_row) {
                if line.trim().is_empty() {
                    break;
                }
                end_row += 1;
            } else {
                break;
            }
        }

        let start_pos = Position::new(start_row, 0);
        let end_pos = if end_row < buffer.lines.len() {
            Position::new(end_row, 0)
        } else {
            // End of buffer
            let last_line = buffer
                .lines
                .get(buffer.lines.len() - 1)
                .map(|line| line.len())
                .unwrap_or(0);
            Position::new(buffer.lines.len() - 1, last_line)
        };

        // For "around" mode, include the trailing empty line if it exists
        let final_end = if mode == TextObjectMode::Around && end_row < buffer.lines.len() {
            Position::new(end_row + 1, 0)
        } else {
            end_pos
        };

        Ok(Some(TextObjectRange::new(
            start_pos,
            final_end,
            TextObjectType::Paragraph,
            mode,
        )))
    }

    /// Find quoted text object
    fn find_quote(
        &self,
        buffer: &Buffer,
        cursor: Position,
        mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        let line = match buffer.lines.get(cursor.row) {
            Some(line) => line,
            None => return Ok(None),
        };

        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            return Ok(None);
        }

        // Look for quotes around cursor position
        for &quote_char in &['"', '\'', '`'] {
            if let Some(range) = self.find_quote_pair(&chars, cursor.col, quote_char, mode)? {
                let start_pos = Position::new(cursor.row, range.0);
                let end_pos = Position::new(cursor.row, range.1);
                return Ok(Some(TextObjectRange::new(
                    start_pos,
                    end_pos,
                    TextObjectType::Quote,
                    mode,
                )));
            }
        }

        Ok(None)
    }

    /// Find bracket pair text object
    fn find_bracket_pair(
        &self,
        buffer: &Buffer,
        cursor: Position,
        mode: TextObjectMode,
        open_char: char,
        close_char: char,
    ) -> Result<Option<TextObjectRange>> {
        // For now, implement single-line bracket matching
        // This could be enhanced for multi-line bracket matching
        let line = match buffer.lines.get(cursor.row) {
            Some(line) => line,
            None => return Ok(None),
        };

        let chars: Vec<char> = line.chars().collect();
        if let Some(range) =
            self.find_matching_brackets(&chars, cursor.col, open_char, close_char, mode)?
        {
            let start_pos = Position::new(cursor.row, range.0);
            let end_pos = Position::new(cursor.row, range.1);

            let object_type = match open_char {
                '(' => TextObjectType::Paren,
                '[' => TextObjectType::Bracket,
                '{' => TextObjectType::Brace,
                '<' => TextObjectType::Angle,
                _ => TextObjectType::Paren,
            };

            return Ok(Some(TextObjectRange::new(
                start_pos,
                end_pos,
                object_type,
                mode,
            )));
        }

        Ok(None)
    }

    /// Find HTML/XML tag text object
    fn find_tag(
        &self,
        _buffer: &Buffer,
        _cursor: Position,
        _mode: TextObjectMode,
    ) -> Result<Option<TextObjectRange>> {
        // This is a placeholder for tag finding
        // A full implementation would need to parse HTML/XML properly
        // For now, return None (not implemented)
        debug!("Tag text object not yet implemented");
        Ok(None)
    }

    // Helper methods

    fn is_word_separator(&self, ch: char) -> bool {
        self.word_separators.contains(ch)
    }

    fn find_word_start(&self, chars: &[char], pos: usize) -> usize {
        let mut start = pos;
        while start > 0 && !self.is_word_separator(chars[start - 1]) {
            start -= 1;
        }
        start
    }

    fn find_word_end(&self, chars: &[char], pos: usize) -> usize {
        let mut end = pos;
        while end < chars.len() && !self.is_word_separator(chars[end]) {
            end += 1;
        }
        end
    }

    fn find_word2_start(&self, chars: &[char], pos: usize) -> usize {
        let mut start = pos;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        start
    }

    fn find_word2_end(&self, chars: &[char], pos: usize) -> usize {
        let mut end = pos;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        end
    }

    fn find_whitespace_around(
        &self,
        buffer: &Buffer,
        cursor: Position,
    ) -> Result<Option<TextObjectRange>> {
        let line = match buffer.lines.get(cursor.row) {
            Some(line) => line,
            None => return Ok(None),
        };

        let chars: Vec<char> = line.chars().collect();
        let mut start = cursor.col;
        let mut end = cursor.col;

        // Extend backward
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }

        // Extend forward
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }

        if start < end {
            Ok(Some(TextObjectRange::new(
                Position::new(cursor.row, start),
                Position::new(cursor.row, end),
                TextObjectType::Word, // Generic type for whitespace
                TextObjectMode::Around,
            )))
        } else {
            Ok(None)
        }
    }

    fn find_sentence_start(&self, buffer: &Buffer, cursor: Position) -> Result<Option<Position>> {
        // Simple implementation: start of current line or after previous sentence
        let current_row = cursor.row;
        let line = match buffer.lines.get(current_row) {
            Some(line) => line,
            None => return Ok(None),
        };

        let chars: Vec<char> = line.chars().collect();
        let mut pos = cursor.col;

        // Go backward to find sentence start
        while pos > 0 {
            if self.sentence_enders.contains(chars[pos - 1]) {
                // Found sentence end, start after it (skip whitespace)
                while pos < chars.len() && chars[pos].is_whitespace() {
                    pos += 1;
                }
                return Ok(Some(Position::new(current_row, pos)));
            }
            pos -= 1;
        }

        // Beginning of line
        Ok(Some(Position::new(current_row, 0)))
    }

    fn find_sentence_end(&self, buffer: &Buffer, cursor: Position) -> Result<Option<Position>> {
        let current_row = cursor.row;
        let line = match buffer.lines.get(current_row) {
            Some(line) => line,
            None => return Ok(None),
        };

        let chars: Vec<char> = line.chars().collect();
        let mut pos = cursor.col;

        // Go forward to find sentence end
        while pos < chars.len() {
            if self.sentence_enders.contains(chars[pos]) {
                return Ok(Some(Position::new(current_row, pos + 1)));
            }
            pos += 1;
        }

        // End of line
        Ok(Some(Position::new(current_row, chars.len())))
    }

    fn find_quote_pair(
        &self,
        chars: &[char],
        cursor_col: usize,
        quote_char: char,
        mode: TextObjectMode,
    ) -> Result<Option<(usize, usize)>> {
        // Find matching quote pairs
        let mut quotes = Vec::new();
        for (i, &ch) in chars.iter().enumerate() {
            if ch == quote_char {
                quotes.push(i);
            }
        }

        // Find the pair that contains or is near the cursor
        for chunk in quotes.chunks(2) {
            if chunk.len() == 2 {
                let start = chunk[0];
                let end = chunk[1];

                if cursor_col >= start && cursor_col <= end {
                    return match mode {
                        TextObjectMode::Inner => Ok(Some((start + 1, end))),
                        TextObjectMode::Around => Ok(Some((start, end + 1))),
                    };
                }
            }
        }

        Ok(None)
    }

    fn find_matching_brackets(
        &self,
        chars: &[char],
        cursor_col: usize,
        open_char: char,
        close_char: char,
        mode: TextObjectMode,
    ) -> Result<Option<(usize, usize)>> {
        // Simple bracket matching within a line
        let mut stack = Vec::new();
        let mut pairs = Vec::new();

        for (i, &ch) in chars.iter().enumerate() {
            if ch == open_char {
                stack.push(i);
            } else if ch == close_char {
                if let Some(start) = stack.pop() {
                    pairs.push((start, i));
                }
            }
        }

        // Find the pair that contains the cursor
        for &(start, end) in &pairs {
            if cursor_col >= start && cursor_col <= end {
                return match mode {
                    TextObjectMode::Inner => Ok(Some((start + 1, end))),
                    TextObjectMode::Around => Ok(Some((start, end + 1))),
                };
            }
        }

        Ok(None)
    }
}

/// Parse text object from string (e.g., "iw", "aw", "ip")
pub fn parse_text_object(s: &str) -> Option<(TextObjectMode, TextObjectType)> {
    if s.len() < 2 {
        return None;
    }

    let mode = match s.chars().next()? {
        'i' => TextObjectMode::Inner,
        'a' => TextObjectMode::Around,
        _ => return None,
    };

    let object_type = match s.chars().nth(1)? {
        'w' => TextObjectType::Word,
        'W' => TextObjectType::Word2,
        's' => TextObjectType::Sentence,
        'p' => TextObjectType::Paragraph,
        '"' | '\'' | '`' => TextObjectType::Quote,
        '(' | ')' | 'b' => TextObjectType::Paren, // 'b' is traditional vim for ()
        '[' | ']' => TextObjectType::Bracket,
        '{' | '}' | 'B' => TextObjectType::Brace, // 'B' is traditional vim for {}
        '<' | '>' => TextObjectType::Angle,
        't' => TextObjectType::Tag,
        _ => return None,
    };

    Some((mode, object_type))
}
