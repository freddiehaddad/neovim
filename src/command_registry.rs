use once_cell::sync::Lazy;
use std::collections::HashMap;

/// ex commands + aliases
pub static EX_COMMANDS: Lazy<HashMap<&'static str, Vec<&'static str>>> = Lazy::new(|| {
    let mut m = HashMap::new();

    // Preliminary file/session commands for setup
    m.insert("q", vec!["quit"]);
    m.insert("q!", vec!["quit!"]);
    m.insert("w", vec!["write"]);
    m.insert("wq", vec!["x"]);
    m.insert("e", vec!["edit"]); // open file
    m.insert("b", vec!["buffer"]); // switch buffer
    m.insert("bn", vec!["bnext"]);
    m.insert("bp", vec!["bprev", "bprevious"]);
    m.insert("bd", vec!["bdelete"]);
    m.insert("ls", vec!["buffers"]);

    // Window/split management
    m.insert("split", vec!["sp"]);
    m.insert("vsplit", vec!["vsp"]);
    m.insert("close", vec![]);

    m
});

/// Setting variants for `:set` (name + shorthand)
pub struct SettingVariant {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

/// List of current `:set` options
pub static SETTING_VARIANTS: &[SettingVariant] = &[
    SettingVariant { name: "number", aliases: &["nu"] },
    SettingVariant { name: "relativenumber", aliases: &["rnu"] },
    SettingVariant { name: "cursorline", aliases: &["cul"] },
    SettingVariant { name: "syntax", aliases: &["syn"] },
    SettingVariant { name: "colorscheme", aliases: &["colo"] },
    SettingVariant { name: "ignorecase", aliases: &["ic"] },
    SettingVariant { name: "smartcase", aliases: &["scs"] },
    SettingVariant { name: "hlsearch", aliases: &["hls"] },
    SettingVariant { name: "incsearch", aliases: &["is"] },
    SettingVariant { name: "wrap", aliases: &[] },
    SettingVariant { name: "linebreak", aliases: &["lbr"] },
    SettingVariant { name: "tabstop", aliases: &["ts"] },
    SettingVariant { name: "expandtab", aliases: &["et"] },
    SettingVariant { name: "autoindent", aliases: &["ai"] },
    SettingVariant { name: "backup", aliases: &["bk"] },
    SettingVariant { name: "swapfile", aliases: &["swf"] },
    SettingVariant { name: "autosave", aliases: &["aw"] },
    SettingVariant { name: "undolevels", aliases: &["ul"] },
    SettingVariant { name: "undofile", aliases: &["udf"] },
    SettingVariant { name: "laststatus", aliases: &["ls"] },
    SettingVariant { name: "showcmd", aliases: &["sc"] },
    SettingVariant { name: "timeoutlen", aliases: &["tm"] },
];

/// Expand to all possible `:set` tokens like "number", "nu", "nonumber", "nonu", etc.
pub fn all_set_tokens() -> Vec<String> {
    let mut v = Vec::new();
    for variant in SETTING_VARIANTS {
        v.push(variant.name.to_string());
        v.extend(variant.aliases.iter().map(|a| a.to_string()));
        v.push(format!("no{}", variant.name));
        for a in variant.aliases {
            v.push(format!("no{}", a));
        }
    }
    v
}

pub fn suggest_ex_commands(prefix: &str) -> Vec<String> {
    let lower = prefix.trim();
    let mut matches = Vec::new();
    for (canon, aliases) in EX_COMMANDS.iter() {
        if canon.starts_with(lower) {
            matches.push(canon.to_string());
        }
        for &a in aliases {
            if a.starts_with(lower) {
                matches.push(a.to_string());
            }
        }
    }
    matches.sort();
    matches.dedup();
    matches
}

pub fn suggest_set_args(prefix: &str) -> Vec<String> {
    let trimmed = prefix.trim();
    all_set_tokens()
        .into_iter()
        .filter(|s| s.starts_with(trimmed))
        .collect()
}

/// UI entry point to fetch auto-complete
pub fn suggest(full_input: &str) -> Vec<String> {
    let without_colon = full_input.strip_prefix(':').unwrap_or(full_input);
    if let Some(rest) = without_colon.strip_prefix("set ") {
        // after ":set "
        suggest_set_args(rest)
    } else {
        // completing the ex command
        suggest_ex_commands(without_colon)
    }
}
