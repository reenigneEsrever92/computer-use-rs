//! Keyboard input: turning text and key names into X11 keysyms.
//!
//! The portal injects *keysyms*, not characters, so both tools that produce
//! keystrokes translate first. The rules are old and slightly odd, which is why
//! they live here as pure functions with tests rather than inside the backend:
//!
//! - Printable ASCII and Latin-1 map to their own code point.
//! - Everything else uses the Unicode convention, `0x01000000 | codepoint`.
//! - Named keys — `enter`, `tab`, the arrows — have their own keysyms.
//!
//! A key combination is written the way people write shortcuts, `ctrl+shift+t`,
//! and parses into modifiers plus the key they modify.

/// An X11 keysym.
pub type Keysym = u32;

/// The base of the function-key keysyms, `F1`.
const FUNCTION_KEY_BASE: Keysym = 0xffbe;

/// The offset at which the Unicode convention starts.
const UNICODE_BASE: Keysym = 0x0100_0000;

/// Shift, left.
pub const SHIFT: Keysym = 0xffe1;
/// Control, left.
pub const CONTROL: Keysym = 0xffe3;
/// Alt, left.
pub const ALT: Keysym = 0xffe9;
/// Super, left — the Windows or Command key.
pub const SUPER: Keysym = 0xffeb;

/// The keysym for a character.
///
/// Latin-1 has a keysym per code point; anything beyond it uses the convention
/// that carries the code point in the low bits, which is how a compositor can
/// type a character its keyboard layout has never heard of.
pub fn character_keysym(character: char) -> Keysym {
    let code = character as u32;
    match code {
        0x20..=0x7e | 0xa0..=0xff => code,
        _ => UNICODE_BASE | code,
    }
}

/// The keysyms for a string, in order.
pub fn text_keysyms(text: &str) -> Vec<Keysym> {
    text.chars().map(character_keysym).collect()
}

/// A key with the modifiers to hold while it is pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combination {
    /// Modifiers to press first, in the order given.
    pub modifiers: Vec<Keysym>,
    /// The key itself.
    pub key: Keysym,
}

/// The keysym for a named key, such as `enter` or `f5`.
fn named_keysym(name: &str) -> Option<Keysym> {
    let keysym = match name {
        "enter" | "return" => 0xff0d,
        "tab" => 0xff09,
        "escape" | "esc" => 0xff1b,
        "space" => 0x20,
        "backspace" => 0xff08,
        "delete" | "del" => 0xffff,
        "insert" | "ins" => 0xff63,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" | "page_up" | "pgup" => 0xff55,
        "pagedown" | "page_down" | "pgdn" => 0xff56,
        "up" => 0xff52,
        "down" => 0xff54,
        "left" => 0xff51,
        "right" => 0xff53,
        "menu" => 0xff67,
        "pause" => 0xff13,
        "print" | "printscreen" | "print_screen" => 0xff61,
        "scrolllock" | "scroll_lock" => 0xff14,
        "capslock" | "caps_lock" => 0xffe5,
        "plus" => 0x2b,
        "minus" => 0x2d,
        "comma" => 0x2c,
        "period" => 0x2e,
        "slash" => 0x2f,
        "semicolon" => 0x3b,
        "apostrophe" | "quote" => 0x27,
        "grave" | "backtick" => 0x60,
        "bracketleft" => 0x5b,
        "bracketright" => 0x5d,
        "backslash" => 0x5c,
        "equal" => 0x3d,
        _ => return function_keysym(name).or_else(|| single_character(name)),
    };
    Some(keysym)
}

/// The keysym for `f1` through `f35`, if that is what the name is.
fn function_keysym(name: &str) -> Option<Keysym> {
    let number: u32 = name.strip_prefix('f')?.parse().ok()?;
    (1..=35)
        .contains(&number)
        .then(|| FUNCTION_KEY_BASE + number - 1)
}

/// The keysym for a one-character name, so `a` and `7` and `+` work.
fn single_character(name: &str) -> Option<Keysym> {
    let mut characters = name.chars();
    let character = characters.next()?;
    characters
        .next()
        .is_none()
        .then(|| character_keysym(character))
}

/// The modifier keysym for a name.
fn modifier_keysym(name: &str) -> Option<Keysym> {
    Some(match name {
        "ctrl" | "control" => CONTROL,
        "shift" => SHIFT,
        "alt" | "option" | "opt" => ALT,
        "super" | "meta" | "win" | "cmd" | "command" => SUPER,
        _ => return None,
    })
}

/// The modifier keysyms in a list such as `ctrl+shift`.
///
/// The click, drag, and scroll tools take modifiers as their own argument rather
/// than a held key, so this is what turns that argument into the keysyms the
/// backend presses around the action. An empty string means no modifiers, but a
/// non-empty one with a name that is not a modifier is an error — silently
/// dropping it would send a plain click where a modified one was asked for.
pub fn parse_modifiers(text: &str) -> Result<Vec<Keysym>, String> {
    let mut keysyms = Vec::new();
    for part in text
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let name = part.to_lowercase();
        let keysym = modifier_keysym(&name)
            .ok_or_else(|| format!("`{part}` is not a modifier; try ctrl, shift, alt, or super"))?;
        keysyms.push(keysym);
    }
    Ok(keysyms)
}

/// Parse a combination such as `ctrl+shift+t`.
///
/// Names are case-insensitive, and `plus` is how to write the `+` key, since a
/// literal one would be read as another separator.
pub fn parse_combination(text: &str) -> Result<Combination, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the key combination is empty".to_string());
    }

    let parts: Vec<&str> = text
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();

    let Some((key, modifiers)) = parts.split_last() else {
        return Err(format!("`{text}` is not a key combination"));
    };

    let mut modifier_keysyms = Vec::with_capacity(modifiers.len());
    for modifier in modifiers {
        let name = modifier.to_lowercase();
        let keysym = modifier_keysym(&name).ok_or_else(|| {
            format!("`{modifier}` is not a modifier; try ctrl, shift, alt, or super")
        })?;
        modifier_keysyms.push(keysym);
    }

    let name = key.to_lowercase();
    // A bare modifier is a key in its own right — pressing `super` alone opens an
    // overview — so a name that is only a modifier is accepted as the key rather
    // than rejected as an incomplete combination.
    let keysym = named_keysym(&name)
        .or_else(|| modifier_keysym(&name))
        .ok_or_else(|| format!("`{key}` is not a key this server knows"))?;

    Ok(Combination {
        modifiers: modifier_keysyms,
        key: keysym,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_ascii_maps_to_itself() {
        assert_eq!(character_keysym('a'), 0x61);
        assert_eq!(character_keysym('A'), 0x41);
        assert_eq!(character_keysym('7'), 0x37);
        assert_eq!(character_keysym(' '), 0x20);
    }

    #[test]
    fn latin1_maps_to_itself() {
        assert_eq!(character_keysym('é'), 0xe9);
        assert_eq!(character_keysym('ü'), 0xfc);
    }

    #[test]
    fn anything_beyond_latin1_uses_the_unicode_convention() {
        assert_eq!(character_keysym('€'), 0x0100_20ac);
        assert_eq!(character_keysym('→'), 0x0100_2192);
        assert_eq!(character_keysym('😀'), 0x0101_f600);
    }

    #[test]
    fn text_maps_character_by_character() {
        assert_eq!(text_keysyms("hi"), vec![0x68, 0x69]);
        assert_eq!(text_keysyms("aé€"), vec![0x61, 0xe9, 0x0100_20ac]);
        assert!(text_keysyms("").is_empty());
    }

    #[test]
    fn a_bare_key_parses_with_no_modifiers() {
        let combination = parse_combination("a").expect("a combination");
        assert!(combination.modifiers.is_empty());
        assert_eq!(combination.key, 0x61);
    }

    #[test]
    fn modifiers_parse_in_order() {
        let combination = parse_combination("ctrl+shift+t").expect("a combination");
        assert_eq!(combination.modifiers, vec![CONTROL, SHIFT]);
        assert_eq!(combination.key, 0x74);
    }

    #[test]
    fn parsing_ignores_case_and_spacing() {
        let combination = parse_combination(" Ctrl + Shift + T ").expect("a combination");
        assert_eq!(combination.modifiers, vec![CONTROL, SHIFT]);
        // Names are case-insensitive, so the key is the lowercase one; shift is
        // what makes it an uppercase character on screen.
        assert_eq!(combination.key, 0x74);
    }

    #[test]
    fn named_keys_have_their_own_keysyms() {
        assert_eq!(parse_combination("enter").expect("enter").key, 0xff0d);
        assert_eq!(parse_combination("Escape").expect("escape").key, 0xff1b);
        assert_eq!(parse_combination("f5").expect("f5").key, 0xffbe + 4);
        assert_eq!(parse_combination("up").expect("up").key, 0xff52);
    }

    #[test]
    fn the_plus_key_is_written_as_a_word() {
        let combination = parse_combination("ctrl+plus").expect("a combination");
        assert_eq!(combination.modifiers, vec![CONTROL]);
        assert_eq!(combination.key, 0x2b);
    }

    #[test]
    fn a_bare_modifier_is_a_key_of_its_own() {
        // Pressing super alone is how an overview opens, so a lone modifier has to
        // be usable as the key rather than rejected as an incomplete combination.
        for name in ["super", "meta", "win", "cmd", "command"] {
            let combination = parse_combination(name).expect(name);
            assert!(combination.modifiers.is_empty(), "{name}");
            assert_eq!(combination.key, SUPER, "{name}");
        }

        assert_eq!(parse_combination("ctrl").expect("ctrl").key, CONTROL);
    }

    #[test]
    fn an_unknown_modifier_is_rejected_by_name() {
        let error = parse_combination("hyper+t").expect_err("an error");
        assert!(error.contains("hyper"), "{error}");
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        let error = parse_combination("ctrl+nonesuch").expect_err("an error");
        assert!(error.contains("nonesuch"), "{error}");
    }

    #[test]
    fn modifiers_parse_as_a_plus_joined_list() {
        assert_eq!(
            parse_modifiers("ctrl+shift").expect("modifiers"),
            vec![CONTROL, SHIFT]
        );
        assert_eq!(parse_modifiers(" shift ").expect("modifiers"), vec![SHIFT]);
        assert_eq!(parse_modifiers("super").expect("modifiers"), vec![SUPER]);
    }

    #[test]
    fn an_empty_modifier_list_means_none() {
        assert!(parse_modifiers("").expect("modifiers").is_empty());
        assert!(parse_modifiers("   ").expect("modifiers").is_empty());
    }

    #[test]
    fn an_unknown_modifier_is_rejected_rather_than_dropped() {
        // Dropping it silently would send an unmodified click where the caller
        // asked for a modified one, which is the failure that matters here.
        let error = parse_modifiers("ctrl+nonesuch").expect_err("an error");
        assert!(error.contains("nonesuch"), "{error}");
    }

    #[test]
    fn an_empty_combination_is_rejected() {
        assert!(parse_combination("").is_err());
        assert!(parse_combination("   ").is_err());
    }
}
