//! Keyboard shortcuts, matched on every modifier.

use eframe::egui;

/// Takes a press of `key` made with exactly `modifiers` held, and says whether
/// there was one.
///
/// egui's own `consume_key` ignores a Shift or an Alt the shortcut does not
/// name, so asking for Ctrl+S takes Ctrl+Shift+S as well — which is how Save As
/// came to write a file without asking where. egui's answer is to ask for the
/// most specific shortcut first, a rule that holds only for as long as every
/// list of shortcuts is kept in that order by hand, and both applications' lists
/// had drifted out of it. A shortcut here is its modifiers as much as its key,
/// as it is in Word and Excel: Ctrl+Z does not answer for Ctrl+Shift+Z, in any
/// order.
pub fn take(input: &mut egui::InputState, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    let before = input.events.len();
    input.events.retain(|event| {
        !matches!(
            event,
            egui::Event::Key {
                key: pressed,
                modifiers: held,
                pressed: true,
                ..
            } if *pressed == key && held.matches_exact(modifiers)
        )
    });
    let taken = input.events.len() != before;
    // A letter pressed with Alt and Shift still sends its text on X11, and a
    // key taken as a shortcut must not also be typed: Alt+Shift+C opened the
    // Review pane and put a "C" in the document beside it. The text that
    // rides with the key goes with it.
    if taken && modifiers.alt {
        let name = key.name();
        input.events.retain(
            |event| !matches!(event, egui::Event::Text(text) if text.eq_ignore_ascii_case(name)),
        );
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressing(key: egui::Key, modifiers: egui::Modifiers) -> egui::InputState {
        let mut input = egui::InputState::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        input
    }

    #[test]
    fn an_extra_shift_is_a_different_shortcut() {
        let ctrl_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let mut input = pressing(egui::Key::S, ctrl_shift);
        assert!(
            !take(&mut input, egui::Modifiers::COMMAND, egui::Key::S),
            "Ctrl+S does not answer for Ctrl+Shift+S"
        );
        assert!(
            take(&mut input, ctrl_shift, egui::Key::S),
            "and the press is still there for the shortcut it was"
        );
        assert!(
            !take(&mut input, ctrl_shift, egui::Key::S),
            "and taking it takes it once"
        );
    }

    #[test]
    fn a_key_taken_with_alt_takes_the_letter_it_typed_with_it() {
        let alt_shift = egui::Modifiers::ALT | egui::Modifiers::SHIFT;
        let mut input = pressing(egui::Key::C, alt_shift);
        input.events.push(egui::Event::Text("C".to_owned()));
        assert!(take(&mut input, alt_shift, egui::Key::C));
        assert!(
            !input
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(_))),
            "the C is not left to be typed"
        );
        // A plain letter's text stays: taking Ctrl+B leaves a typed "b" alone,
        // and there is none on any platform anyway.
        let mut input = pressing(egui::Key::B, egui::Modifiers::COMMAND);
        input.events.push(egui::Event::Text("x".to_owned()));
        assert!(take(&mut input, egui::Modifiers::COMMAND, egui::Key::B));
        assert_eq!(input.events.len(), 1);
    }

    #[test]
    fn an_extra_alt_is_a_different_shortcut() {
        let ctrl_alt = egui::Modifiers::COMMAND | egui::Modifiers::ALT;
        let mut input = pressing(egui::Key::M, ctrl_alt);
        assert!(!take(&mut input, egui::Modifiers::COMMAND, egui::Key::M));
        assert!(take(&mut input, ctrl_alt, egui::Key::M));
    }
}
