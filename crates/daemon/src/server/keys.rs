//! What a client's keys do: the prefix, the direct and the lock layers, the prompt line, and the
//! keys that go on to the program in the pane.

use super::*;
use crate::copy::JumpKey;
use crate::keymap::{self, Binding, Keymap, typed_name};
use crate::keytools;
use crate::picker::PickerKind;
use crate::status::{MessageKind, Prompt};
use arcade_term::Mods;

use super::treeview::TreeStart;

/// How long a message stays on the status bar when no key is pressed.
const MESSAGE_LIFETIME: Duration = Duration::from_secs(3);

/// How long the prefix waits for a key before the hint of the keys shows.
const HINT_DELAY: Duration = Duration::from_millis(500);

/// Where the prefix table stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrefixState {
    Idle,
    /// The prefix key was pressed; the next key is looked up in the prefix table.
    Awaiting,
    /// The modifier of the prefix key is still held (a key arrived with it): keys that carry it
    /// are looked up in the prefix table without it, until one comes without it.
    Holding,
    /// The swap key was pressed; the next movement key says which neighbour to swap with.
    /// `held`: the modifier of the prefix key is still held.
    Swapping {
        held: bool,
    },
}

/// The modifiers of the prefix key that a person can hold down while pressing other keys.
fn held_mods(prefix: Key) -> Mods {
    Mods {
        ctrl: prefix.mods.ctrl,
        alt: prefix.mods.alt,
        super_key: prefix.mods.super_key,
        ..Mods::NONE
    }
}

/// The key carries every held modifier of the prefix (and there is one to hold).
fn carries(key: Key, held: Mods) -> bool {
    held != Mods::NONE
        && (!held.ctrl || key.mods.ctrl)
        && (!held.alt || key.mods.alt)
        && (!held.super_key || key.mods.super_key)
}

/// The key as the prefix table knows it, with the held modifiers taken off.
fn without_held(key: Key, held: Mods) -> Key {
    keymap::normalize(Key {
        code: key.code,
        mods: Mods {
            ctrl: key.mods.ctrl && !held.ctrl,
            alt: key.mods.alt && !held.alt,
            super_key: key.mods.super_key && !held.super_key,
            shift: key.mods.shift,
        },
    })
}

fn is_bare_escape(key: Key) -> bool {
    key.code == Code::Named(Named::Escape) && key.mods == Mods::NONE
}

/// The command a swap key stands for, in the table of keys after the prefix.
const SWAP_MODE: &str = "pane.swap-mode";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    /// Closing the focused pane, answered with `y` or `n`.
    ClosePane,
    RenameWindow,
    RenameSession,
    /// The one argument of the named command, typed on the prompt line.
    Argument(&'static str),
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
    /// Whether the line is news or an error, which sets its colour.
    pub message_kind: MessageKind,
    /// When the key hint shows, while the prefix is waiting for a key and it is not showing yet.
    pub hint_at: Option<Instant>,
    /// The hint of the keys is on the screen.
    pub hint_shown: bool,
}

impl KeyState {
    pub fn new() -> KeyState {
        KeyState {
            prefix: PrefixState::Idle,
            locked: false,
            prompt: None,
            message: None,
            message_kind: MessageKind::default(),
            message_until: None,
            hint_at: None,
            hint_shown: false,
        }
    }

    /// The key hint is on the screen.
    pub fn hint_visible(&self) -> bool {
        match self.prefix {
            PrefixState::Idle => false,
            PrefixState::Awaiting => self.hint_shown,
            PrefixState::Holding | PrefixState::Swapping { .. } => true,
        }
    }

    /// Show a line on the status bar for `MESSAGE_LIFETIME`, or until the next key.
    pub fn show_message(&mut self, text: String, kind: MessageKind, now: Instant) {
        self.message = Some(text);
        self.message_kind = kind;
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
        let hint = self
            .hint_at
            .filter(|_| self.prefix == PrefixState::Awaiting);
        [self.message_until, hint].into_iter().flatten().min()
    }

    pub fn status_prompt(&self) -> Option<Prompt> {
        self.prompt.as_ref().map(|p| Prompt {
            label: p.label.clone(),
            text: p.text.clone(),
            question: p.kind == PromptKind::ClosePane,
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
        if let Some(pane) = self.focus_pane_of(session_id)
            && let Some(capture) = self.panes.get_mut(&pane).and_then(|p| p.capture.as_mut())
        {
            capture.record(crate::capture::Direction::Host, bytes);
        }
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
                Input::Mouse(mouse) => self.handle_mouse(session_id, mouse),
                Input::Focus(focused) => self.host_focus(session_id, focused),
                Input::Raw(raw) => {
                    if self.host_reply(session_id, &raw) {
                        continue;
                    }
                    if self.in_prompt(session_id)
                        || self.picker_open(session_id)
                        || self.focus_in_copy_mode(session_id)
                    {
                        continue;
                    }
                    if let Some(pane) = self.focus_pane_of(session_id) {
                        let flags = self
                            .panes
                            .get(&pane)
                            .map_or(0, |p| p.term.key_modes().kitty_flags);
                        // What a pane gets must mean what a key means; the rest is dropped.
                        if !crate::keys::fit_for_pane(&raw, flags) {
                            continue;
                        }
                        self.typed_into(pane);
                        self.write_to_pane(pane, &raw);
                    }
                }
            }
        }
    }

    /// An answer to the question of the host terminal's keyboard, asked at attach: it is for
    /// Arcade, not for the pane. When the last answer is in, a terminal that cannot send all the
    /// keys the tables use is told on the status bar.
    fn host_reply(&mut self, session_id: u64, raw: &[u8]) -> bool {
        let Some(reply) = keytools::read_reply(raw) else {
            return false;
        };
        let Some(s) = self.sessions.get_mut(&session_id) else {
            return false;
        };
        let Some(kitty) = s.host_probe else {
            // Nothing was asked: it is the pane's own business.
            return false;
        };
        match reply {
            keytools::Reply::Keyboard => s.host_probe = Some(true),
            keytools::Reply::Attributes => {
                s.host_probe = None;
                let host = if kitty {
                    keytools::Host::Kitty
                } else {
                    keytools::Host::Legacy
                };
                s.host_keyboard = Some(host);
                let client = s.client;
                if let Some(c) = client.and_then(|id| self.clients.get_mut(&id)) {
                    c.host_keyboard = Some(host);
                }
                if let Some(line) = self.key_warning_line(session_id) {
                    self.set_error(session_id, &line);
                }
            }
        }
        true
    }

    /// The one line about bindings that cannot work, when the host is known and some cannot.
    pub(super) fn key_warning_line(&self, session_id: u64) -> Option<String> {
        let host = self.sessions.get(&session_id)?.host_keyboard?;
        keytools::status_line(&keytools::warnings(&self.config, Some(host)))
    }

    pub(super) fn picker_open(&self, session_id: u64) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|s| s.picker.is_some() || s.tree.is_some())
    }

    /// The focused pane shows copy mode, so keys are for copy mode and not for its program.
    fn focus_in_copy_mode(&self, session_id: u64) -> bool {
        self.focus_pane_of(session_id)
            .and_then(|p| self.panes.get(&p))
            .is_some_and(|p| p.copy.is_some())
    }

    pub(super) fn in_prompt(&self, session_id: u64) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|s| s.key_state.prompt.is_some())
    }

    /// The prefix window ended: leave the prefix table.
    pub(super) fn expire_key_states(&mut self, now: Instant) {
        for s in self.sessions.values_mut() {
            if s.key_state.message_until.is_some_and(|t| now >= t) && s.key_state.clear_message() {
                s.chrome_dirty = true;
            }
            if s.key_state.prefix == PrefixState::Awaiting
                && s.key_state.hint_at.is_some_and(|t| now >= t)
            {
                s.key_state.hint_at = None;
                s.key_state.hint_shown = true;
                s.chrome_dirty = true;
            }
        }
    }

    pub(super) fn set_message(&mut self, session_id: u64, text: &str) {
        self.show(session_id, text, MessageKind::Information);
    }

    /// A line on the status bar that says something did not work.
    pub(super) fn set_error(&mut self, session_id: u64, text: &str) {
        self.show(session_id, text, MessageKind::Error);
    }

    fn show(&mut self, session_id: u64, text: &str, kind: MessageKind) {
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.key_state
                .show_message(text.to_string(), kind, Instant::now());
            s.chrome_dirty = true;
        }
    }

    fn set_prefix(&mut self, session_id: u64, state: PrefixState) {
        if let Some(s) = self.sessions.get_mut(&session_id)
            && s.key_state.prefix != state
        {
            s.key_state.prefix = state;
            s.chrome_dirty = true;
            match state {
                PrefixState::Awaiting => {
                    s.key_state.hint_at = Some(Instant::now() + HINT_DELAY);
                    s.key_state.hint_shown = false;
                }
                // Someone who holds the modifier is looking for the next key: the hint stays
                // up until the modifier is let go.
                PrefixState::Holding | PrefixState::Swapping { .. } => {
                    s.key_state.hint_at = None;
                    s.key_state.hint_shown = true;
                }
                PrefixState::Idle => {
                    s.key_state.hint_at = None;
                    s.key_state.hint_shown = false;
                }
            }
        }
    }

    fn handle_key(&mut self, session_id: u64, original: Key, raw: &[u8]) {
        // Tables are looked up by the normal form; a pane gets the key as it came (`:` and
        // not Shift and `;`).
        let key = keymap::normalize(original);
        let Some(s) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if s.key_state.clear_message() {
            s.chrome_dirty = true;
        }
        if let Some(tree) = s.tree.as_mut() {
            if let Some(b) = Keymap::get(&self.keymap.list, key).cloned() {
                self.run_binding(session_id, &b);
            } else if let Some(text) = typed_text(key, raw) {
                tree.notice = None;
                tree.type_text(text);
                s.relayout = true;
                s.chrome_dirty = true;
            }
            return;
        }
        if let Some(picker) = s.picker.as_mut() {
            // The keys that only the tree has are nothing here: a space is typed into the filter.
            let bound = Keymap::get(&self.keymap.list, key).filter(|b| {
                !matches!(b.command.as_str(), "list.left" | "list.right" | "list.lift")
            });
            if let Some(b) = bound.cloned() {
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
        if state != PrefixState::Idle
            && self.prefix_table_key(session_id, state, key, original, raw)
        {
            return;
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

    /// A key while the prefix table is in effect. `false` when the key ended the holding and is
    /// typing after all: it goes on to the direct layer and the pane, not eaten.
    fn prefix_table_key(
        &mut self,
        session_id: u64,
        state: PrefixState,
        key: Key,
        original: Key,
        raw: &[u8],
    ) -> bool {
        let Some(pane) = self.focus_pane_of(session_id) else {
            return true;
        };
        let prefix = keymap::normalize(self.config.keys.prefix);
        if key == prefix {
            // The prefix twice sends the prefix key itself.
            self.set_prefix(session_id, PrefixState::Idle);
            self.key_to_focus(session_id, pane, original, raw);
            return true;
        }
        let held = held_mods(prefix);
        // A terminal with old key codes cannot say whether Ctrl is down: one command a prefix.
        let can_hold = self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.host_keyboard != Some(keytools::Host::Legacy));
        let carried = can_hold && carries(key, held);
        let looked = if carried {
            without_held(key, held)
        } else {
            key
        };
        let bound = Keymap::get(&self.keymap.prefix, looked).cloned();
        match state {
            PrefixState::Awaiting => {
                if bound.is_none() && is_bare_escape(key) {
                    self.set_prefix(session_id, PrefixState::Idle);
                    return true;
                }
                let next = if carried {
                    PrefixState::Holding
                } else {
                    PrefixState::Idle
                };
                match bound {
                    Some(b) if b.command == SWAP_MODE => {
                        self.set_prefix(session_id, PrefixState::Swapping { held: carried });
                    }
                    Some(b) => {
                        self.set_prefix(session_id, next);
                        self.run_binding(session_id, &b);
                    }
                    None => {
                        self.set_prefix(session_id, next);
                        self.set_error(session_id, &format!("no binding for {}", typed_name(key)));
                    }
                }
                true
            }
            PrefixState::Holding => {
                if !carried {
                    // The modifier was let go: this key is typing.
                    self.set_prefix(session_id, PrefixState::Idle);
                    return false;
                }
                match bound {
                    Some(b) if b.command == SWAP_MODE => {
                        self.set_prefix(session_id, PrefixState::Swapping { held: true });
                    }
                    Some(b) => self.run_binding(session_id, &b),
                    // Still held: a key that means nothing here is passed over.
                    None => {}
                }
                true
            }
            PrefixState::Swapping { held: was_held } => {
                if was_held && !carried {
                    // The modifier was let go while the swap waited: this key is typing.
                    self.set_prefix(session_id, PrefixState::Idle);
                    return false;
                }
                if is_bare_escape(key) {
                    self.set_prefix(session_id, PrefixState::Idle);
                    return true;
                }
                let next = if was_held || carried {
                    PrefixState::Holding
                } else {
                    PrefixState::Idle
                };
                let direction = bound
                    .as_ref()
                    .and_then(|b| b.command.strip_prefix("pane.focus-"))
                    .filter(|d| matches!(*d, "left" | "down" | "up" | "right"));
                match direction {
                    Some(d) => {
                        self.set_prefix(session_id, next);
                        let (ok, output) =
                            self.run_command(&format!("pane.swap-{d}"), &[], Some(pane));
                        if !ok {
                            self.set_error(session_id, &output);
                        }
                    }
                    None => {
                        self.set_prefix(session_id, next);
                        self.set_error(
                            session_id,
                            &format!("{} is not a direction", typed_name(key)),
                        );
                    }
                }
                true
            }
            PrefixState::Idle => false,
        }
    }

    /// A key no table above took: for the pane in copy mode it is a copy mode key (or nothing),
    /// for any other pane it is typing.
    fn key_to_focus(&mut self, session_id: u64, pane: u64, original: Key, raw: &[u8]) {
        let in_copy = self.panes.get(&pane).is_some_and(|p| p.copy.is_some());
        if !in_copy {
            self.send_key_to_pane(pane, original, raw);
            return;
        }
        // While a jump is open every key belongs to it.
        let jumping = self
            .panes
            .get(&pane)
            .and_then(|p| p.copy.as_ref())
            .is_some_and(|c| c.jump.is_some());
        if jumping {
            let key = match original.code {
                Code::Named(Named::Escape) => Some(JumpKey::Escape),
                Code::Named(Named::Enter) => Some(JumpKey::Enter),
                Code::Named(Named::Backspace) => Some(JumpKey::Backspace),
                _ => typed_text(original, raw)
                    .and_then(|t| t.chars().next())
                    .map(JumpKey::Char),
            };
            if let Some(key) = key
                && let Some(p) = self.panes.get_mut(&pane)
                && let Some(copy) = p.copy.as_mut()
            {
                copy.jump_key(&p.term, key);
                if let Some(s) = self.sessions.get_mut(&session_id) {
                    s.chrome_dirty = true;
                }
            }
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
            ("window.select", true) => {
                Some((PromptKind::Argument("window.select"), "Window number: "))
            }
            ("session.switch", true) => Some((
                PromptKind::Argument("session.switch"),
                "Switch to session: ",
            )),
            ("copy.search-forward", _) if self.focus_in_copy_mode(session_id) => {
                Some((PromptKind::SearchForward, "/"))
            }
            ("copy.search-backward", _) if self.focus_in_copy_mode(session_id) => {
                Some((PromptKind::SearchBackward, "?"))
            }
            _ => None,
        };
        let tree_start = match binding.command.as_str() {
            "tree.open" => Some(TreeStart::Browse),
            "pane.move" => Some(TreeStart::LiftPane),
            "window.move" => Some(TreeStart::LiftWindow),
            _ => None,
        };
        if let Some(start) = tree_start.filter(|_| binding.args.is_empty()) {
            if let Err(message) = self.open_tree(session_id, start) {
                self.set_error(session_id, &message);
            }
            return;
        }
        let picker = match (binding.command.as_str(), binding.args.is_empty()) {
            ("session.list", true) => Some(PickerKind::Sessions),
            ("window.list", true) => Some(PickerKind::Windows),
            ("window.move-to-session", true) => Some(PickerKind::MoveWindow),
            ("keys.list", true) => Some(PickerKind::Keys),
            ("command.list", true) => Some(PickerKind::Commands),
            ("notification.list", true) => Some(PickerKind::Notifications),
            _ => None,
        };
        if let Some(kind) = picker {
            if let Err(message) = self.open_picker(session_id, kind) {
                self.set_error(session_id, &message);
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
        if !ok {
            self.set_error(session_id, &output);
        } else if binding.command == "arcade.reload-config" {
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
            // An empty window name is an answer: the window goes back to automatic naming.
            PromptKind::RenameSession
            | PromptKind::Argument(_)
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
            PromptKind::Argument(name) => (name, vec![prompt.text]),
        };
        let (ok, output) = self.run_command(name, &args, Some(pane));
        if !ok {
            self.set_error(session_id, &output);
        }
    }

    fn handle_paste(&mut self, session_id: u64, text: &[u8]) {
        if self
            .sessions
            .get(&session_id)
            .is_some_and(|s| s.picker.is_some() || s.tree.is_some())
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
        self.typed_into(pane_id);
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
        self.typed_into(pane_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_waited_for_when_idle_and_the_hint_is_a_single_deadline() {
        let mut state = KeyState::new();
        assert_eq!(state.deadline(), None);
        let at = Instant::now() + HINT_DELAY;
        state.prefix = PrefixState::Awaiting;
        state.hint_at = Some(at);
        assert_eq!(state.deadline(), Some(at));
        // Once the hint is up there is nothing more to wait for.
        state.hint_at = None;
        state.hint_shown = true;
        assert_eq!(state.deadline(), None);
        assert!(state.hint_visible());
        // A hint time left behind by a prefix that ended is not a deadline.
        state.prefix = PrefixState::Idle;
        state.hint_at = Some(at);
        assert_eq!(state.deadline(), None);
        assert!(!state.hint_visible());
    }
}
