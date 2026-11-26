//! Command-line arguments. There is exactly one thing to ask for: the version.
//!
//! Hand-rolled rather than pulled from a crate: one flag does not justify an
//! argument-parsing dependency in a binary whose whole appeal is that it is a
//! single file with nothing underneath it.

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Print the version and exit.
    ShowVersion,
    /// Start the UI — the default, and what anything unrecognised falls back to.
    Run,
}

/// Read the arguments (without the program name).
pub fn parse<I, S>(args: I) -> Action
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for arg in args {
        if matches!(arg.as_ref(), "-v" | "--version") {
            return Action::ShowVersion;
        }
    }
    Action::Run
}

/// The line `-v` prints: the usual `name version` shape, so `--version | cut`
/// and friends behave the way they do for every other tool.
pub fn version_line() -> String {
    format!("git-branch-manager {}", crate::update::CURRENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_spellings_ask_for_the_version() {
        assert_eq!(parse(["-v"]), Action::ShowVersion);
        assert_eq!(parse(["--version"]), Action::ShowVersion);
    }

    #[test]
    fn no_arguments_starts_the_ui() {
        let none: [&str; 0] = [];
        assert_eq!(parse(none), Action::Run);
    }

    #[test]
    fn anything_else_starts_the_ui_as_it_always_has() {
        // There is no argument handling beyond this flag, and adding one here
        // would turn today's "ignored" into an error.
        assert_eq!(parse(["--colour=always"]), Action::Run);
        assert_eq!(parse(["/some/path"]), Action::Run);
    }

    #[test]
    fn the_flag_is_found_wherever_it_sits() {
        assert_eq!(parse(["--other", "-v"]), Action::ShowVersion);
    }

    #[test]
    fn the_printed_version_is_the_one_the_update_check_compares() {
        let line = version_line();
        assert_eq!(
            line,
            format!("git-branch-manager {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(
            line.ends_with(crate::update::CURRENT),
            "a version printed here but not used there would make `-v` lie about updates"
        );
    }
}
