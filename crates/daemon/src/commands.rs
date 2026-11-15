//! The named commands: from a name and its arguments to the operation. Keys, the `arcade run`
//! command and the configuration file all go through this one list, so that no operation can
//! be done by one of them only.

use crate::layout::{Axis, Direction};

/// The commands of copy mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyCommand {
    Left,
    Down,
    Up,
    Right,
    WordNext,
    WordBack,
    WordEnd,
    LineStart,
    LineFirstChar,
    LineEnd,
    Top,
    Bottom,
    HalfPageUp,
    HalfPageDown,
    PageUp,
    PageDown,
    SearchForward,
    SearchBackward,
    SearchNext,
    SearchPrevious,
    SelectChar,
    SelectLine,
    Yank,
    YankStay,
    OpenLink,
    /// Open a jump: type what to go to, then the label of a match.
    Jump,
    Cancel,
    Exit,
}

/// The commands of the floating list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListCommand {
    Up,
    Down,
    Select,
    Cancel,
    /// Take the last typed character off the filter, or all of it.
    FilterDelete,
    FilterClear,
    /// In the tree: out to the item this one is inside, into it, and lift it or put it down.
    Left,
    Right,
    Lift,
    /// In the list of notifications: clear the one on the chosen line.
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    PaneFocus(Direction),
    PaneFocusLast,
    PaneResize(Direction),
    PaneSwap(Direction),
    /// The key that starts a swap: the next movement key says with which neighbour.
    PaneSwapMode,
    PaneSplit(Axis),
    PaneZoom,
    PaneClose,
    PanePaste,
    PaneBreak,
    WindowNew,
    WindowSelect(usize),
    WindowNext,
    WindowPrevious,
    WindowLast,
    WindowRename(Option<String>),
    WindowMoveLeft,
    WindowMoveRight,
    WindowMoveToSession(Option<String>),
    WindowList,
    /// Open the tree to look and jump, with a pane lifted, or with a window lifted.
    TreeOpen,
    PaneMove,
    WindowMove,
    WindowLayoutNext,
    WindowLayoutEven,
    SessionList,
    SessionSwitch(String),
    SessionRename(Option<String>),
    SessionDetach,
    SessionClose(Option<String>),
    KeysList,
    Palette,
    CopyEnter,
    Copy(CopyCommand),
    List(ListCommand),
    /// Go to the pane whose notification is the most urgent (and has waited longest), skipping
    /// the pane that is looked at.
    NotificationNext,
    NotificationList,
    /// Clear the notification of the pane that has the focus, or of every pane.
    NotificationClear,
    NotificationClearAll,
    ArcadeLock,
    ArcadeReloadConfig,
    ArcadeRedraw,
}

/// Every command name with the command it makes when it takes no argument (or a placeholder
/// argument). Used to check names in the configuration file.
const NAMES: &[(&str, Command)] = &[
    ("pane.focus-left", Command::PaneFocus(Direction::Left)),
    ("pane.focus-down", Command::PaneFocus(Direction::Down)),
    ("pane.focus-up", Command::PaneFocus(Direction::Up)),
    ("pane.focus-right", Command::PaneFocus(Direction::Right)),
    ("pane.focus-last", Command::PaneFocusLast),
    ("pane.resize-left", Command::PaneResize(Direction::Left)),
    ("pane.resize-down", Command::PaneResize(Direction::Down)),
    ("pane.resize-up", Command::PaneResize(Direction::Up)),
    ("pane.resize-right", Command::PaneResize(Direction::Right)),
    ("pane.swap-mode", Command::PaneSwapMode),
    ("pane.swap-left", Command::PaneSwap(Direction::Left)),
    ("pane.swap-down", Command::PaneSwap(Direction::Down)),
    ("pane.swap-up", Command::PaneSwap(Direction::Up)),
    ("pane.swap-right", Command::PaneSwap(Direction::Right)),
    ("pane.split-right", Command::PaneSplit(Axis::LeftRight)),
    ("pane.split-down", Command::PaneSplit(Axis::TopBottom)),
    ("pane.zoom", Command::PaneZoom),
    ("pane.close", Command::PaneClose),
    ("pane.paste", Command::PanePaste),
    ("pane.break", Command::PaneBreak),
    ("window.new", Command::WindowNew),
    ("window.next", Command::WindowNext),
    ("window.previous", Command::WindowPrevious),
    ("window.last", Command::WindowLast),
    ("window.rename", Command::WindowRename(None)),
    ("window.move-left", Command::WindowMoveLeft),
    ("window.move-right", Command::WindowMoveRight),
    ("window.move-to-session", Command::WindowMoveToSession(None)),
    ("window.list", Command::WindowList),
    ("window.layout-next", Command::WindowLayoutNext),
    ("window.layout-even", Command::WindowLayoutEven),
    ("session.list", Command::SessionList),
    ("session.rename", Command::SessionRename(None)),
    ("session.detach", Command::SessionDetach),
    ("session.close", Command::SessionClose(None)),
    ("copy.enter", Command::CopyEnter),
    ("copy.left", Command::Copy(CopyCommand::Left)),
    ("copy.down", Command::Copy(CopyCommand::Down)),
    ("copy.up", Command::Copy(CopyCommand::Up)),
    ("copy.right", Command::Copy(CopyCommand::Right)),
    ("copy.word-next", Command::Copy(CopyCommand::WordNext)),
    ("copy.word-back", Command::Copy(CopyCommand::WordBack)),
    ("copy.word-end", Command::Copy(CopyCommand::WordEnd)),
    ("copy.line-start", Command::Copy(CopyCommand::LineStart)),
    (
        "copy.line-first-char",
        Command::Copy(CopyCommand::LineFirstChar),
    ),
    ("copy.line-end", Command::Copy(CopyCommand::LineEnd)),
    ("copy.top", Command::Copy(CopyCommand::Top)),
    ("copy.bottom", Command::Copy(CopyCommand::Bottom)),
    ("copy.half-page-up", Command::Copy(CopyCommand::HalfPageUp)),
    (
        "copy.half-page-down",
        Command::Copy(CopyCommand::HalfPageDown),
    ),
    ("copy.page-up", Command::Copy(CopyCommand::PageUp)),
    ("copy.page-down", Command::Copy(CopyCommand::PageDown)),
    (
        "copy.search-forward",
        Command::Copy(CopyCommand::SearchForward),
    ),
    (
        "copy.search-backward",
        Command::Copy(CopyCommand::SearchBackward),
    ),
    ("copy.search-next", Command::Copy(CopyCommand::SearchNext)),
    (
        "copy.search-previous",
        Command::Copy(CopyCommand::SearchPrevious),
    ),
    ("copy.select-char", Command::Copy(CopyCommand::SelectChar)),
    ("copy.select-line", Command::Copy(CopyCommand::SelectLine)),
    ("copy.yank", Command::Copy(CopyCommand::Yank)),
    ("copy.yank-stay", Command::Copy(CopyCommand::YankStay)),
    ("copy.open-link", Command::Copy(CopyCommand::OpenLink)),
    ("copy.jump", Command::Copy(CopyCommand::Jump)),
    ("copy.cancel", Command::Copy(CopyCommand::Cancel)),
    ("copy.exit", Command::Copy(CopyCommand::Exit)),
    ("list.up", Command::List(ListCommand::Up)),
    ("list.down", Command::List(ListCommand::Down)),
    ("list.select", Command::List(ListCommand::Select)),
    ("list.cancel", Command::List(ListCommand::Cancel)),
    (
        "list.filter-delete",
        Command::List(ListCommand::FilterDelete),
    ),
    ("list.filter-clear", Command::List(ListCommand::FilterClear)),
    ("list.left", Command::List(ListCommand::Left)),
    ("list.right", Command::List(ListCommand::Right)),
    ("list.lift", Command::List(ListCommand::Lift)),
    ("list.clear", Command::List(ListCommand::Clear)),
    ("notification.next", Command::NotificationNext),
    ("notification.list", Command::NotificationList),
    ("notification.clear", Command::NotificationClear),
    ("notification.clear-all", Command::NotificationClearAll),
    ("tree.open", Command::TreeOpen),
    ("pane.move", Command::PaneMove),
    ("window.move", Command::WindowMove),
    ("keys.list", Command::KeysList),
    ("command.list", Command::Palette),
    ("arcade.lock", Command::ArcadeLock),
    ("arcade.reload-config", Command::ArcadeReloadConfig),
    ("arcade.redraw", Command::ArcadeRedraw),
];

/// Commands that take an argument, and so are not in [`NAMES`] with a plain form.
#[cfg_attr(not(test), allow(dead_code))] // the configuration file checks names with it
const WITH_ARGUMENT: &[&str] = &["window.select", "session.switch"];

/// Whether `name` is a command of the key and configuration surface.
#[cfg_attr(not(test), allow(dead_code))] // the configuration file checks names with it
pub fn is_known(name: &str) -> bool {
    NAMES.iter().any(|(n, _)| *n == name) || WITH_ARGUMENT.contains(&name)
}

/// Every name, for listings and for suggesting a near match.
#[cfg_attr(not(test), allow(dead_code))] // the configuration file suggests near matches with it
pub fn all_names() -> impl Iterator<Item = &'static str> {
    NAMES
        .iter()
        .map(|(n, _)| *n)
        .chain(WITH_ARGUMENT.iter().copied())
}

/// Make the command a name and its arguments stand for.
pub fn parse(name: &str, args: &[String]) -> Result<Command, String> {
    let optional = || args.first().cloned();
    match name {
        "window.select" => {
            let number = args
                .first()
                .ok_or_else(|| "window.select needs a window number".to_string())?;
            let number: usize = number
                .parse()
                .map_err(|_| format!("not a window number: {number}"))?;
            Ok(Command::WindowSelect(number))
        }
        "session.switch" => args
            .first()
            .cloned()
            .map(Command::SessionSwitch)
            .ok_or_else(|| "session.switch needs a session name".to_string()),
        "window.rename" => Ok(Command::WindowRename(optional())),
        "window.move-to-session" => Ok(Command::WindowMoveToSession(optional())),
        "session.rename" => Ok(Command::SessionRename(optional())),
        "session.close" => Ok(Command::SessionClose(optional())),
        other => NAMES
            .iter()
            .find(|(n, _)| *n == other)
            .map(|(_, c)| c.clone())
            .ok_or_else(|| format!("unknown command: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_parses_and_unknown_ones_do_not() {
        for name in all_names() {
            assert!(is_known(name));
            let args = if name == "window.select" {
                vec!["1".to_string()]
            } else if name == "session.switch" {
                vec!["a".to_string()]
            } else {
                Vec::new()
            };
            assert!(parse(name, &args).is_ok(), "{name}");
        }
        assert!(parse("pane.fly", &[]).is_err());
        assert!(!is_known("pane.fly"));
        assert!(parse("window.select", &[]).is_err());
        assert!(parse("window.select", &["x".to_string()]).is_err());
        assert_eq!(
            parse("window.select", &["3".to_string()]),
            Ok(Command::WindowSelect(3))
        );
        assert_eq!(
            parse("window.rename", &["tên".to_string()]),
            Ok(Command::WindowRename(Some("tên".to_string())))
        );
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = all_names().collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
