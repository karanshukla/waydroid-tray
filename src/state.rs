//! The session state the tray shows.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Stopped,
    /// Waydroid's session manager is up, but the container has no session yet.
    Starting,
    Running,
    Frozen,
    /// The container is holding on to a session nothing owns any more, left
    /// behind by a stop that failed partway. Waydroid refuses new starts
    /// ("Already tracking a session") until it's stopped again.
    Stuck,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Stopped => "Stopped",
            State::Starting => "Starting...",
            State::Running => "Running",
            State::Frozen => "Frozen (idle)",
            State::Stuck => "Stuck (stop the session to reset)",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            State::Stopped | State::Starting | State::Stuck => "waydroid-tray-stopped-symbolic",
            State::Running => "waydroid-tray-running-symbolic",
            State::Frozen => "waydroid-tray-frozen-symbolic",
        }
    }

    /// Pinned by the `with_session` tests below.
    pub fn with_session(self, session_up: bool) -> State {
        if session_up && self == State::Stopped {
            State::Starting
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_with_session_manager_up_is_starting() {
        assert_eq!(State::Stopped.with_session(true), State::Starting);
    }

    #[test]
    fn other_states_are_unchanged_by_the_session_manager() {
        for state in [State::Running, State::Frozen, State::Stuck, State::Starting] {
            assert_eq!(state.with_session(true), state);
        }
        assert_eq!(State::Stopped.with_session(false), State::Stopped);
    }
}
