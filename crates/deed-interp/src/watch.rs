//! Somewhere to stand while a program runs.
//!
//! A debugger needs two things a compiler does not: a place where execution
//! can be observed, and a way to hold it there. The evaluator is recursive and
//! single pass, so there is no state machine to suspend. What there is, at
//! every statement, is a point where nothing is half done.
//!
//! So the interpreter offers that point and nothing else. It calls
//! [`Watcher::at`] and carries on when the call returns. A watcher that wants
//! to stop simply does not return yet, which is the whole of suspension: the
//! host stack is the program's stack, and leaving it alone is what makes the
//! state a debugger reads the state the program is actually in.
//!
//! Nothing here decides anything. Breakpoints, stepping, and what a step even
//! means are the caller's, because the interpreter cannot be right about them
//! and would only be a second place to be wrong. See
//! `design/decisions/2026-08-04-a-place-to-stand.md`.

use deed_diagnostics::{FileId, Span};

use crate::interp::Interp;

/// One active call, as a debugger sees it.
///
/// Values are rendered here rather than handed out live. A [`Value`] is full
/// of `Rc`, so it belongs to the thread that made it, and a debugger that
/// serves a protocol is usually somewhere else. Text crosses; a reference
/// count does not.
///
/// [`Value`]: crate::Value
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameView {
    /// What the function is called where it is written.
    pub function: String,
    /// The module it is written in.
    pub module: String,
    pub file: FileId,
    /// The statement this call is at, which for every frame but the innermost
    /// is the one it is waiting on.
    pub span: Span,
    /// Every name this call can see, and what it is bound to, by name.
    pub variables: Vec<(String, String)>,
}

/// Where a run has got to, and what may be asked about it.
///
/// The cheap answers are fields; [`Paused::stack`] walks every frame and
/// renders every binding, so it is a method and is only paid for by a watcher
/// that stops. A watcher that is running to a breakpoint asks the line and
/// returns, and that is the case that happens millions of times.
pub struct Paused<'p, 'a> {
    pub(crate) interp: &'p Interp<'a>,
    pub(crate) file: FileId,
    pub(crate) span: Span,
}

impl Paused<'_, '_> {
    /// The file the next statement is in.
    pub fn file(&self) -> FileId {
        self.file
    }

    /// The statement about to run.
    pub fn span(&self) -> Span {
        self.span
    }

    /// How many calls are active, `main` being one.
    ///
    /// This is what stepping is defined against: over is "not deeper than
    /// here", out is "shallower than here", and in is "anywhere".
    pub fn depth(&self) -> usize {
        self.interp.active_calls()
    }

    /// Every active call, innermost first.
    pub fn stack(&self) -> Vec<FrameView> {
        self.interp.stack_view(self.file, self.span)
    }

    /// Lines written through a `Console` so far.
    ///
    /// A run collects its output and hands it over at the end, which is fine
    /// for a test and useless for somebody watching. Reading it at each stop
    /// is how a debugger can show what has been printed by the time the
    /// program got here.
    pub fn output(&self) -> &[String] {
        self.interp.written()
    }
}

/// Something that watches a program run.
///
/// One method, because there is one thing the interpreter can offer. Whether
/// this is a breakpoint, a step, or a program being single stepped from the
/// first line is a question the watcher answers with its own state.
pub trait Watcher {
    /// Called before each statement, with the program held still.
    ///
    /// Execution continues when this returns, so a watcher stops by not
    /// returning. It must not run Deed code: the interpreter is in the middle
    /// of a call and is lent out, not free.
    fn at(&mut self, paused: Paused<'_, '_>);
}
