// Search and replace functionality
// This will handle pattern matching, regex, and text search

use log::{debug, info};
use regex::Regex;

pub struct SearchEngine {
    last_search: Option<String>,
    case_sensitive: bool,
    use_regex: bool,
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            last_search: None,
            case_sensitive: false,
            use_regex: false,
        }
    }

    pub fn set_case_sensitive(&mut self, case_sensitive: bool) {
        self.case_sensitive = case_sensitive;
    }

    pub fn set_use_regex(&mut self, use_regex: bool) {
        self.use_regex = use_regex;
    }

    pub fn search(&mut self, pattern: &str, text: &[String]) -> Vec<SearchResult> {
        info!(
            "Performing search for pattern: '{}' (case_sensitive: {}, use_regex: {})",
            pattern, self.case_sensitive, self.use_regex
        );
        self.last_search = Some(pattern.to_string());

        let mut results = Vec::new();

        if self.use_regex {
            if let Ok(regex) = Regex::new(pattern) {
                for (line_num, line) in text.iter().enumerate() {
                    for mat in regex.find_iter(line) {
                        results.push(SearchResult {
                            line: line_num,
                            start_col: mat.start(),
                            end_col: mat.end(),
                            matched_text: mat.as_str().to_string(),
                        });
                    }
                }
            }
        } else {
            // Simple string search
            let search_pattern = if self.case_sensitive {
                pattern.to_string()
            } else {
                pattern.to_lowercase()
            };

            for (line_num, line) in text.iter().enumerate() {
                let search_line = if self.case_sensitive {
                    line.clone()
                } else {
                    line.to_lowercase()
                };

                let mut start = 0;
                while let Some(pos) = search_line[start..].find(&search_pattern) {
                    let actual_pos = start + pos;
                    results.push(SearchResult {
                        line: line_num,
                        start_col: actual_pos,
                        end_col: actual_pos + pattern.len(),
                        matched_text: line[actual_pos..actual_pos + pattern.len()].to_string(),
                    });
                    start = actual_pos + 1;
                }
            }
        }

        debug!("Search completed, found {} matches", results.len());
        results
    }

    pub fn replace(&self, _pattern: &str, _replacement: &str, _text: &mut [String]) -> usize {
        // TODO: Implement replace functionality
        0
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub matched_text: String,
}
