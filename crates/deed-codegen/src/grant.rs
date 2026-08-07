//! What a host grants a compiled program, and how the program names it.
//!
//! [`crate::run::Host`] says which imports are answered at all. This says
//! what the answers are made of: the capabilities an embedder decided to
//! hand over, and the handles the program refers to them by.
//!
//! # Handles
//!
//! A capability crosses the boundary as a plain number, and that is the
//! point. The number is an index into a table the host keeps; the thing it
//! stands for lives on the host's side and is never written into the
//! module's memory. So a compiled program cannot look inside a capability,
//! cannot widen one, and cannot build one out of arithmetic: every operation
//! that acts on a capability looks its argument up in the table first, and a
//! number the host never handed out is not a capability, it is a number, and
//! the call stops with [`Trap::Refused`].
//!
//! That check is not there for Deed programs. A checked Deed program cannot
//! reach it: a capability is a value with a type and nothing in the language
//! turns an integer into one. It is there because a host is handed modules
//! rather than programs, and a host whose safety rests on the last compiler
//! that touched the module is not enforcing anything.
//!
//! # Nothing by default
//!
//! [`Grants::none`] offers nothing, and each grant adds exactly the imports
//! it can answer. A program that writes to a console it was not granted is
//! refused by [`crate::run::Host::link`] before a single instruction runs,
//! naming the import that went unanswered. That is the same sentence a
//! component gets from a real engine, and it is the reason the grants are
//! spelled out here rather than assumed: what a host does not name, a
//! program does not get.
//!
//! See `design/decisions/2026-08-07-what-a-host-hands-a-compiled-program.md`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::run::{Host, HostCall, Trap, Value};

/// Something the host handed out, by what it is.
///
/// The tag is what makes a handle unforgeable in the useful direction: a
/// module that passes the console where a clock was wanted is refused, so
/// the type the source language checked is checked again at the boundary
/// where the source language stops being in charge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Held {
    /// The root a program's `main` is handed.
    System,
    /// The console lines are written to.
    Console,
    /// The clock time is read from.
    Clock,
}

impl Held {
    /// What to call this in a refusal.
    fn name(self) -> &'static str {
        match self {
            Held::System => "System",
            Held::Console => "Console",
            Held::Clock => "Clock",
        }
    }
}

/// Where a written line goes.
type Sink = Box<dyn FnMut(&str)>;

/// What an embedder decided a compiled program may reach.
///
/// Built up a grant at a time and turned into a [`Host`] by
/// [`Grants::into_host`].
#[derive(Default)]
pub struct Grants {
    console: Option<Sink>,
    clock: bool,
}

/// A host built from a set of grants, with the handle `main` is handed.
pub struct Granted {
    /// The host to link the module against.
    pub host: Host,
    /// The `System` handle, which is what every parameter of a compiled
    /// `main` is: the interpreter hands `main` the root capability for each
    /// one it declares, and this is the same root on the other side.
    pub system: Value,
}

impl Grants {
    /// A host that grants nothing.
    pub fn none() -> Self {
        Self::default()
    }

    /// Grant a console, and say where a written line goes.
    ///
    /// One call per `Io.write`, with the text and without a terminator:
    /// which character a line ends with is a fact about the machine it is
    /// printed on rather than about the program that wrote it.
    pub fn console(mut self, write: impl FnMut(&str) + 'static) -> Self {
        self.console = Some(Box::new(write));
        self
    }

    /// Grant a clock.
    ///
    /// `Io.now` counts, `Io.epoch` reads the machine. The same split the
    /// interpreter makes and for the same reason: one of them can give the
    /// same answer twice and the other cannot, and the row says which is
    /// being asked for.
    pub fn clock(mut self) -> Self {
        self.clock = true;
        self
    }

    /// Build the host that answers for these grants.
    pub fn into_host(self) -> Granted {
        let mut held = vec![Held::System];
        let has_console = self.console.is_some();
        if has_console {
            held.push(Held::Console);
        }
        if self.clock {
            held.push(Held::Clock);
        }

        let table = Rc::new(RefCell::new(Table {
            held,
            console: self.console,
            ticks: 0,
        }));
        let system = table
            .borrow()
            .handle(Held::System)
            .expect("the root is always in the table");

        let mut host = Host::new();
        if has_console {
            let narrow = Rc::clone(&table);
            host.offer("deed:sys", "console", move |call| {
                let table = narrow.borrow();
                table.require(&call, Held::System, "sys.console")?;
                Ok(table.handle(Held::Console))
            });

            let write = Rc::clone(&table);
            host.offer("deed:io", "write", move |call| {
                let mut table = write.borrow_mut();
                table.require(&call, Held::Console, "Io.write")?;
                let line = call.text(1).ok_or_else(|| {
                    Trap::Refused(
                        "`Io.write` was handed something that is not a string".to_string(),
                    )
                })?;
                if let Some(console) = table.console.as_mut() {
                    console(&line);
                }
                Ok(None)
            });
        }

        if self.clock {
            let narrow = Rc::clone(&table);
            host.offer("deed:sys", "clock", move |call| {
                let table = narrow.borrow();
                table.require(&call, Held::System, "sys.clock")?;
                Ok(table.handle(Held::Clock))
            });

            let now = Rc::clone(&table);
            host.offer("deed:io", "now", move |call| {
                let mut table = now.borrow_mut();
                table.require(&call, Held::Clock, "Io.now")?;
                table.ticks += 1;
                Ok(Some(Value::I64(table.ticks)))
            });

            let epoch = Rc::clone(&table);
            host.offer("deed:io", "epoch", move |call| {
                epoch.borrow().require(&call, Held::Clock, "Io.epoch")?;
                Ok(Some(Value::I64(millis_since_epoch())))
            });
        }

        Granted { host, system }
    }
}

/// The handles this host has handed out, and what they stand for.
struct Table {
    held: Vec<Held>,
    console: Option<Sink>,
    ticks: i64,
}

impl Table {
    /// The handle for this capability, when it was granted.
    ///
    /// One past the index, so that zero is never a handle. A module that
    /// passes an uninitialised word gets a refusal rather than the console.
    fn handle(&self, what: Held) -> Option<Value> {
        self.held
            .iter()
            .position(|one| *one == what)
            .map(|at| Value::I64(at as i64 + 1))
    }

    /// What a handle the module passed back stands for, if anything.
    fn stands_for(&self, value: Option<Value>) -> Option<Held> {
        let index = usize::try_from(value?.as_i64().checked_sub(1)?).ok()?;
        self.held.get(index).copied()
    }

    /// Check that the capability argument is the one this operation acts on.
    fn require(&self, call: &HostCall<'_>, what: Held, operation: &str) -> Result<(), Trap> {
        if self.stands_for(call.arg(0)) == Some(what) {
            return Ok(());
        }
        Err(Trap::Refused(format!(
            "`{operation}` was handed something that is not a `{}` this host granted",
            what.name()
        )))
    }
}

/// Milliseconds since 1970, negative before it.
///
/// A clock set before 1970 is a machine that is wrong, and the honest number
/// for it is a negative one. The interpreter answers the same way.
fn millis_since_epoch() -> i64 {
    let now = std::time::SystemTime::now();
    match now.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => since.as_millis() as i64,
        Err(before) => -(before.duration().as_millis() as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grants decide the offers, so what nobody granted is not offered
    /// and a module that wants it is refused at link.
    #[test]
    fn nothing_is_granted_by_default() {
        let granted = Grants::none().into_host();
        assert!(
            granted
                .host
                .implementation_for("deed:sys", "console")
                .is_none()
        );
        assert!(
            granted
                .host
                .implementation_for("deed:io", "write")
                .is_none()
        );
        assert!(granted.host.implementation_for("deed:io", "now").is_none());
    }

    #[test]
    fn granting_a_console_offers_writing_and_nothing_else() {
        let granted = Grants::none().console(|_| {}).into_host();
        assert!(
            granted
                .host
                .implementation_for("deed:sys", "console")
                .is_some()
        );
        assert!(
            granted
                .host
                .implementation_for("deed:io", "write")
                .is_some()
        );
        assert!(
            granted
                .host
                .implementation_for("deed:sys", "clock")
                .is_none(),
            "a console is not a clock"
        );
    }

    #[test]
    fn granting_a_clock_offers_both_ways_of_asking_it_the_time() {
        let granted = Grants::none().clock().into_host();
        assert!(granted.host.implementation_for("deed:io", "now").is_some());
        assert!(
            granted
                .host
                .implementation_for("deed:io", "epoch")
                .is_some()
        );
    }

    /// The console handle comes from narrowing the root, and writing to it
    /// reaches the sink the embedder named.
    #[test]
    fn a_line_written_to_the_granted_console_reaches_the_embedder() {
        let written = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&written);
        let granted = Grants::none()
            .console(move |line| sink.borrow_mut().push(line.to_string()))
            .into_host();

        let mut memory = vec![0u8; 256];
        memory[..8].copy_from_slice(&128u64.to_le_bytes());
        let text = HostCall::new(&[], &mut memory)
            .write_text("hello, world")
            .expect("there is room");

        let console = granted
            .host
            .implementation_for("deed:sys", "console")
            .expect("a console was granted")(HostCall::new(
            &[granted.system],
            &mut memory,
        ))
        .expect("narrowing the root is allowed")
        .expect("it answers with a handle");

        granted
            .host
            .implementation_for("deed:io", "write")
            .expect("a console was granted")(HostCall::new(&[console, text], &mut memory))
        .expect("writing to the granted console is allowed");

        assert_eq!(*written.borrow(), vec!["hello, world".to_string()]);
    }

    /// A number the host never handed out is not a capability.
    ///
    /// Nothing a checked Deed program can do, and the whole reason the table
    /// is looked at rather than trusted: a host is handed modules.
    #[test]
    fn a_handle_the_host_never_gave_out_is_refused() {
        let granted = Grants::none().console(|_| {}).into_host();
        let mut memory = vec![0u8; 64];
        let stopped = granted
            .host
            .implementation_for("deed:io", "write")
            .expect("a console was granted")(HostCall::new(
            &[Value::I64(99)],
            &mut memory,
        ))
        .expect_err("99 is not a handle this host gave out");
        assert!(matches!(stopped, Trap::Refused(_)), "{stopped}");
    }

    /// And one it did hand out, for something else.
    ///
    /// The half that makes the test above worth having: refusing every
    /// number except the ones in the table would still let the console be
    /// passed where a clock belongs.
    #[test]
    fn a_handle_for_the_wrong_capability_is_refused() {
        let granted = Grants::none().console(|_| {}).clock().into_host();
        let mut memory = vec![0u8; 64];
        let stopped = granted
            .host
            .implementation_for("deed:io", "now")
            .expect("a clock was granted")(HostCall::new(
            &[granted.system],
            &mut memory,
        ))
        .expect_err("the root is not the clock");
        let Trap::Refused(why) = stopped else {
            panic!("it should be a refusal");
        };
        assert!(
            why.contains("`Clock`"),
            "it should say what it wanted: {why}"
        );
    }

    /// `Io.now` counts rather than reading the machine, so two runs of the
    /// same program agree.
    #[test]
    fn the_counting_clock_counts() {
        let granted = Grants::none().clock().into_host();
        let mut memory = vec![0u8; 64];
        let now = granted
            .host
            .implementation_for("deed:io", "now")
            .expect("a clock was granted");
        let clock = granted
            .host
            .implementation_for("deed:sys", "clock")
            .expect("a clock was granted")(HostCall::new(
            &[granted.system],
            &mut memory,
        ))
        .expect("narrowing the root is allowed")
        .expect("it answers with a handle");

        let first = now(HostCall::new(&[clock], &mut memory)).expect("granted");
        let second = now(HostCall::new(&[clock], &mut memory)).expect("granted");
        assert_eq!(first, Some(Value::I64(1)));
        assert_eq!(second, Some(Value::I64(2)));
    }
}
