//! The session state machine (component-architecture.md, Session manager).

use forge_api::model::SessionState;

/// Whether a session may move from `from` to `to`.
pub fn can_transition(from: SessionState, to: SessionState) -> bool {
    use SessionState::*;
    matches!(
        (from, to),
        (Provisioning, Ready | Failed)
            | (Ready, Working | Done)
            | (Working, Waiting | Ready | Review | Paused | Failed)
            | (Waiting, Working | Paused)
            | (Paused, Working | Done)
            | (Review, Working | Done)
            | (Failed, Provisioning | Done)
    )
}

/// Sessions in these states hold their worktrees and count toward concurrency limits.
pub fn is_live(state: SessionState) -> bool {
    !matches!(state, SessionState::Done | SessionState::Failed)
}

/// States that accept a new prompt.
pub fn accepts_prompt(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Ready | SessionState::Review | SessionState::Paused
    )
}

/// States `session.close` accepts.
pub fn can_close(state: SessionState) -> bool {
    can_transition(state, SessionState::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionState::*;

    const ALL: [SessionState; 8] = [
        Provisioning,
        Ready,
        Working,
        Waiting,
        Paused,
        Review,
        Done,
        Failed,
    ];

    #[test]
    fn transition_table() {
        let allowed: Vec<(SessionState, SessionState)> = ALL
            .iter()
            .flat_map(|&from| ALL.iter().map(move |&to| (from, to)))
            .filter(|&(from, to)| can_transition(from, to))
            .collect();
        assert_eq!(
            allowed,
            [
                (Provisioning, Ready),
                (Provisioning, Failed),
                (Ready, Working),
                (Ready, Done),
                (Working, Ready),
                (Working, Waiting),
                (Working, Paused),
                (Working, Review),
                (Working, Failed),
                (Waiting, Working),
                (Waiting, Paused),
                (Paused, Working),
                (Paused, Done),
                (Review, Working),
                (Review, Done),
                (Failed, Provisioning),
                (Failed, Done),
            ]
        );
    }

    #[test]
    fn prompts_and_close() {
        let prompts: Vec<_> = ALL.into_iter().filter(|s| accepts_prompt(*s)).collect();
        assert_eq!(prompts, [Ready, Paused, Review]);
        let closes: Vec<_> = ALL.into_iter().filter(|s| can_close(*s)).collect();
        assert_eq!(closes, [Ready, Paused, Review, Failed]);
        let live: Vec<_> = ALL.into_iter().filter(|s| is_live(*s)).collect();
        assert_eq!(
            live,
            [Provisioning, Ready, Working, Waiting, Paused, Review]
        );
    }
}
