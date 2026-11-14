//! What a client's keys do: the prefix, the direct and the lock layers, the prompt line, and the
//! keys that go on to the program in the pane.

use super::*;
use crate::keymap::{self, Binding, Keymap, typed_name};
use crate::picker::PickerKind;
use crate::status::Prompt;

/// How long a message stays on the status bar when no key is pressed.
const MESSAGE_LIFETIME: Duration = Duration::from_secs(3);

/// Where the prefix table stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrefixState {
    Idle,
    /// The prefix key was pressed; the next key is looked up in the prefix table.
    Awaiting,
    /// A repeatable binding just ran; keys are looked up in the prefix table until this time.
    Repeating(Instant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    /// Closing the focused pane, answered with `y` or `n`.
    ClosePane,
    RenameWindow,
    RenameSession,
    /// The text to look for in copy mode, going down or going up.
    SearchForward,
    SearchBackward,
}

pub(super) struct PromptState {
    pub kind: PromptKind,
    pub label: String,
    pub text: String,
}

/// Everything about a session's keyboard that is not in the tables.
pub(super) struct KeyState {
    pub prefix: PrefixState,
    pub locked: bool,
    pub prompt: Option<PromptState>,
    /// A line shown in place of the status bar until the next key, or until `message_until`.
    pub message: Option<String>,
    pub message_until: Option<Instant>,
}

impl KeyState {
    pub fn new() -> KeyState {
        KeyState {
            prefix: PrefixState::Idle,
            locked: false,
            prompt: None,
            message: None,
            message_until: None,
        }
    }

    /// Show a line on the status bar for `MESSAGE_LIFETIME`, or until the next key.
    pub fn show_message(&mut self, text: String, now: Instant) {
        self.message = Some(text);
        self.message_until = Some(now + MESSAGE_LIFETIME);
    }

    /// Take the line away; whether there was one.
    pub fn clear_message(&mut self) -> bool {
        self.message_until = None;
        self.message.take().is_some()
    }

    /// The prefix table is the one in effect.
    pub fn prefix_active(&self) -> bool {
        self.prefix != PrefixState::Idle
    }

    /// When the repeat window ends, if one is open.
    pub fn deadline(&self) -> Option<Instant> {
        let repeat = match self.prefix {
            PrefixState::Repeating(t) => Some(t),
            _ => None,
        };
        match (repeat, self.message_until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn status_prompt(&self) -> Option<Prompt> {
        self.prompt.as_ref().map(|p| Prompt {
            label: p.label.clone(),
            text: p.text.clone(),
        })
    }
}

/// The text a key types, if it types any (not a control key and not an escape sequence).
fn typed_text(key: Key, raw: &[u8]) -> Option<&str> {
    std::str::from_utf8(raw)
        .ok()
        .filter(|t| !t.is_empty() && !t.starts_with('\x1b') && !key.mods.ctrl && !key.mods.alt)
        .filter(|t| !t.chars().any(char::is_control))
}

impl Server {
    /// Keyboard bytes from a client, read as keys.
    pub(super) fn handle_input(&mut self, session_id: u64, bytes: &[u8]) {
        let inputs = match self.sessions.get_mut(&session_id) {
            Some(s) => s.keys.feed(bytes, Instant::now()),
            None => return,
        };
        self.dispatch_inputs(session_id, inputs);
    }

    pub(super) fn dispatch_inputs(&mut self, session_id: u64, inputs: Vec<Input>) {
        let client = self.sessions.get(&session_id).and_then(|s| s.client);
        let mut session_id = session_id;
        for input in inputs {
            // A key may have taken the client to another session: the keys after it go there.
            if self.sessions.get(&session_id).and_then(|s| s.client) != client {
                match client.and_then(|c| self.session_of_client(c)) {
                    Some(now) => session_id = now,
                    None => return,
                }
            }
            match input {
                Input::Key { key, raw } => self.handle_key(session_id, key, &raw),
                Input::Paste(text) => self.handle_paste(session_id, &text),
                Input::Raw(raw) => {
                    if self.in_prompt(session_id)
                        || self.picker_open(session_id)
                        || self.focus_in_copy_mode(session_id)
                    {
                        continue;
                    }
                    if let Some(pane) = self.focus_pane_of(session_id) {
                        self.write_to_pane(pane, &raw);
                    }
                }
            }
        }
    }

    fn picker_open(&self, session_id: u64) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|s| s.picker.is_some())
    }

    /// The focused pane shows copy mode, so keys are for copy mode and not for its program.
    fn focus_in_copy_mode(&self, session_id: u64) -> bool {
        self.focus_pane_of(session_id)
            .and_then(|p| self.panes.get(&p))
            .is_some_and(|p| p.copy.is_some())
    }

    fn in_prompt(&self, session_id: u64) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|s| s.key_state.prompt.is_some())
    }

    /// The prefix window ended: leave the prefix table.
    pub(super) fn expire_key_states(&mut self, now: Instant) {
        for s in self.sessions.values_mut() {
            if let PrefixState::Repeating(t) = s.key_state.prefix
                && now >= t
            {
                s.key_state.prefix = PrefixState::Idle;
                s.chrome_dirty = true;
            }
            if s.key_state.message_until.is_some_and(|t| now >= t) && s.key_state.clear_message() {
                s.chrome_dirty = true;
            }
        }
    }

    pub(super) fn set_message(&mut self, session_id: u64, text: &str) {
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.key_state.show_message(text.to_string(), Instant::now());
            s.chrome_dirty = true;
        }
    }

    fn set_prefix(&mut self, session_id: u64, state: PrefixState) {
        if let Some(s) = self.sessions.get_mut(&session_id)
            && s.key_state.prefix != state
        {
            s.key_state.prefix = state;
            s.chrome_dirty = true;
        }
    }

    fn handle_key(&mut self, session_id: u64, original: Key, raw: &[u8]) {
        let now = Instant::now();
        // Tables are looked up by the normal form; a pane gets the key as it came (`:` and
        // not Shift and `;`).
        let key = keymap::normalize(original);
        let Some(s) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if let PrefixState::Repeating(t) = s.key_state.prefix
            && now >= t
        {
            s.key_state.prefix = PrefixState::Idle;
            s.chrome_dirty = true;
        }
        if s.key_state.clear_message() {
            s.chrome_dirty = true;
        }
        if let Some(picker) = s.picker.as_mut() {
            if let Some(b) = Keymap::get(&self.keymap.list, key).cloned() {
                self.run_binding(session_id, &b);
            } else if let Some(text) = typed_text(key, raw) {
                // Anything else that can be typed goes into the filter box.
                picker.type_text(text);
                // The box may be smaller now: what was under it is drawn again.
                s.relayout = true;
                s.chrome_dirty = true;
            }
            return;
        }
        if s.key_state.prompt.is_some() {
            self.prompt_key(session_id, key, raw);
            return;
        }
        let Some(pane) = self.focus_pane_of(session_id) else {
            return;
        };
        let locked = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.key_state.locked);
        if locked {
            match Keymap::get(&self.keymap.direct, key) {
                Some(b) if b.command == "arcade.lock" => {
                    let b = b.clone();
                    self.run_binding(session_id, &b);
                }
                _ => self.key_to_focus(session_id, pane, original, raw),
            }
            return;
        }
        let state = self
            .sessions
            .get(&session_id)
            .map_or(PrefixState::Idle, |s| s.key_state.prefix);
        if state != PrefixState::Idle {
            // While the repeat window is open only a repeatable binding still counts; any other
            // key is typing, and closes the window below.
            let binding = Keymap::get(&self.keymap.prefix, key)
                .filter(|b| state == PrefixState::Awaiting || b.repeat)
                .cloned();
            if let Some(b) = binding {
                let next = if b.repeat {
                    PrefixState::Repeating(now + self.config.keys.repeat_window)
                } else {
                    PrefixState::Idle
                };
                self.set_prefix(session_id, next);
                self.run_binding(session_id, &b);
                return;
            }
            if state == PrefixState::Awaiting {
                self.set_prefix(session_id, PrefixState::Idle);
                if key == keymap::normalize(self.config.keys.prefix) {
                    // The prefix twice sends the prefix key itself.
                    self.key_to_focus(session_id, pane, original, raw);
                } else {
                    self.set_message(session_id, &format!("no binding for {}", typed_name(key)));
                }
                return;
            }
            // The repeat window was open and this key is not in the table: it is typing.
            self.set_prefix(session_id, PrefixState::Idle);
        }
        if key == keymap::normalize(self.config.keys.prefix) {
            self.set_prefix(session_id, PrefixState::Awaiting);
            return;
        }
        if let Some(b) = Keymap::get(&self.keymap.direct, key).cloned() {
            self.run_binding(session_id, &b);
            return;
        }
        self.key_to_focus(session_id, pane, original, raw);
    }

    /// A key no table above took: for the pane in copy mode it is a copy mode key (or nothing),
    /// for any other pane it is typing.
    fn key_to_focus(&mut self, session_id: u64, pane: u64, original: Key, raw: &[u8]) {
        let in_copy = self.panes.get(&pane).is_some_and(|p| p.copy.is_some());
        if !in_copy {
            self.send_key_to_pane(pane, original, raw);
            return;
        }
        if let Some(b) = Keymap::get(&self.keymap.copy, original).cloned() {
            self.run_binding(session_id, &b);
        }
    }

    /// Run what a key is bound to. Closing a pane asks first and renaming asks for the name;
    /// an error shows on the status bar.
    pub(super) fn run_binding(&mut self, session_id: u64, binding: &Binding) {
        let Some(pane) = self.focus_pane_of(session_id) else {
            return;
        };
        let ask = match (binding.command.as_str(), binding.args.is_empty()) {
            ("pane.close", _) => Some((PromptKind::ClosePane, "Close this pane? (y/n) ")),
            ("window.rename", true) => Some((PromptKind::RenameWindow, "Window name: ")),
            ("session.rename", true) => Some((PromptKind::RenameSession, "Session name: ")),
            ("copy.search-forward", _) if self.focus_in_copy_mode(session_id) => {
                Some((PromptKind::SearchForward, "/"))
            }
            ("copy.search-backward", _) if self.focus_in_copy_mode(session_id) => {
                Some((PromptKind::SearchBackward, "?"))
            }
            _ => None,
        };
        let picker = match (binding.command.as_str(), binding.args.is_empty()) {
            ("session.list", true) => Some(PickerKind::Sessions),
            ("window.move-to-session", true) => Some(PickerKind::MoveWindow),
            ("keys.list", true) => Some(PickerKind::Keys),
            _ => None,
        };
        if let Some(kind) = picker {
            if let Err(message) = self.open_picker(session_id, kind) {
                self.set_message(session_id, &message);
            }
            return;
        }
        if let Some((kind, label)) = ask {
            if let Some(s) = self.sessions.get_mut(&session_id) {
                // A rename starts from the name in use, so a small change is a small edit.
                let text = match kind {
                    PromptKind::RenameWindow => s.current_window().name.clone(),
                    PromptKind::RenameSession => s.name.clone(),
                    _ => String::new(),
                };
                s.key_state.prompt = Some(PromptState {
                    kind,
                    label: label.to_string(),
                    text,
                });
                s.chrome_dirty = true;
            }
            return;
        }
        let (ok, output) = self.run_command(&binding.command, &binding.args, Some(pane));
        if !ok || binding.command == "arcade.reload-config" {
            self.set_message(session_id, &output);
        }
    }

    fn close_prompt(&mut self, session_id: u64) -> Option<PromptState> {
        let s = self.sessions.get_mut(&session_id)?;
        s.chrome_dirty = true;
        s.key_state.prompt.take()
    }

    fn prompt_key(&mut self, session_id: u64, key: Key, raw: &[u8]) {
        let Some(s) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(prompt) = s.key_state.prompt.as_mut() else {
            return;
        };
        let confirm = prompt.kind == PromptKind::ClosePane;
        let typed = typed_text(key, raw);
        match key.code {
            Code::Named(Named::Escape) => {
                self.close_prompt(session_id);
            }
            Code::Char('n') if confirm && typed.is_some() => {
                self.close_prompt(session_id);
            }
            Code::Char('y') if confirm && typed.is_some() => {
                self.accept_prompt(session_id);
            }
            _ if confirm => {}
            Code::Named(Named::Enter) => self.accept_prompt(session_id),
            Code::Named(Named::Backspace) => {
                prompt.text.pop();
                s.chrome_dirty = true;
            }
            Code::Char('u') if key.mods.ctrl => {
                prompt.text.clear();
                s.chrome_dirty = true;
            }
            _ => {
                if let Some(text) = typed {
                    prompt.text.push_str(text);
                    s.chrome_dirty = true;
                }
            }
        }
    }

    fn accept_prompt(&mut self, session_id: u64) {
        let Some(prompt) = self.close_prompt(session_id) else {
            return;
        };
        let Some(pane) = self.focus_pane_of(session_id) else {
            return;
        };
        let (name, args): (&str, Vec<String>) = match prompt.kind {
            PromptKind::ClosePane => ("pane.close", Vec::new()),
            PromptKind::RenameWindow
            | PromptKind::RenameSession
            | PromptKind::SearchForward
            | PromptKind::SearchBackward
                if prompt.text.is_empty() =>
            {
                return;
            }
            PromptKind::SearchForward | PromptKind::SearchBackward => {
                let forward = prompt.kind == PromptKind::SearchForward;
                self.copy_search(session_id, pane, &prompt.text, forward);
                return;
            }
            PromptKind::RenameWindow => ("window.rename", vec![prompt.text]),
            PromptKind::RenameSession => ("session.rename", vec![prompt.text]),
        };
        let (ok, output) = self.run_command(name, &args, Some(pane));
        if !ok {
            self.set_message(session_id, &output);
        }
    }

    fn handle_paste(&mut self, session_id: u64, text: &[u8]) {
        if self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.picker.is_some())
        {
            return;
        }
        if let Some(s) = self.sessions.get_mut(&session_id)
            && let Some(prompt) = s.key_state.prompt.as_mut()
        {
            if prompt.kind != PromptKind::ClosePane {
                let line: String = String::from_utf8_lossy(text)
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                prompt.text.push_str(&line);
                s.chrome_dirty = true;
            }
            return;
        }
        if self.focus_in_copy_mode(session_id) {
            return;
        }
        if let Some(pane) = self.focus_pane_of(session_id) {
            self.send_paste_to_pane(pane, text);
        }
    }

    /// Give a key to a pane's program in the form it asked for.
    pub(super) fn send_key_to_pane(&mut self, pane_id: u64, key: Key, raw: &[u8]) {
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let modes = pane.term.key_modes();
        // A key the host sent in the old way reaches a program that did not ask for the new
        // way exactly as it came: nothing is lost, even for bytes no key stands for.
        let kitty_form = raw.starts_with(b"\x1b[") && raw.ends_with(b"u");
        if modes.kitty_flags == 0 && matches!(key.code, Code::Char(_)) && !kitty_form {
            self.write_to_pane(pane_id, raw);
            return;
        }
        let mut bytes = encode_key(key, modes);
        if bytes.is_empty() && !kitty_form {
            bytes = raw.to_vec();
        }
        self.write_to_pane(pane_id, &bytes);
    }

    /// Give pasted text to a pane, between the paste markers when its program asked for them.
    pub(super) fn send_paste_to_pane(&mut self, pane_id: u64, text: &[u8]) {
        let bracketed = self
            .panes
            .get(&pane_id)
            .is_some_and(|p| p.term.mode(arcade_term::Mode::BracketedPaste) == Some(true));
        if bracketed {
            self.write_to_pane(pane_id, b"\x1b[200~");
        }
        self.write_to_pane(pane_id, text);
        if bracketed {
            self.write_to_pane(pane_id, b"\x1b[201~");
        }
    }
}
