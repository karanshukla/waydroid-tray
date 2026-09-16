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
            State::Stopped | State::Starting | State::Stuck => "waydroid-tray-stopped",
            State::Running => "waydroid-tray-running",
            State::Frozen => "waydroid-tray-frozen",
        }
    }

    /// While the session manager is up, "no session" from the container just
    /// means it hasn't started one yet.
    pub fn with_session(self, session_up: bool) -> State {
        if session_up && self == State::Stopped { State::Starting } else { self }
    }
}
