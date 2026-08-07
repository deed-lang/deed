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
use std::path::PathBuf;
use std::rc::Rc;

use crate::run::{Host, HostCall, Trap, Value};

/// Something the host handed out, by what it is.
///
/// The tag is what makes a handle unforgeable in the useful direction: a
/// module that passes the console where a clock was wanted is refused, so
/// the type the source language checked is checked again at the boundary
/// where the source language stops being in charge.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Held {
    /// The root a program's `main` is handed.
    System,
    /// The console lines are written to.
    Console,
    /// The clock time is read from.
    Clock,
    /// A directory, and everything under it.
    ///
    /// The path is on this side of the boundary. The program has the number
    /// and no way to turn it into a path, which is what stops `Io.open`
    /// being a way to name somewhere else.
    Dir(PathBuf),
    /// The hosts a program may reach.
    ///
    /// The set is on this side of the boundary, for the reason the path is:
    /// a program that could read what its `Net` reaches could compare two of
    /// them, and one that could write it could widen one.
    Net(deed_rt::Reach),
}

impl Held {
    /// What to call this in a refusal.
    fn name(&self) -> &'static str {
        match self {
            Held::System => "System",
            Held::Console => "Console",
            Held::Clock => "Clock",
            Held::Dir(_) => "Dir",
            Held::Net(_) => "Net",
        }
    }

    /// Whether this is the kind of thing that one is, ignoring which one.
    fn is(&self, kind: &Held) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(kind)
    }
}

/// Where a written line goes.
type Sink = Box<dyn FnMut(&str)>;

/// What an embedder decided a compiled program may reach.
///
/// Built up a grant at a time and turned into a [`Host`] by
/// [`Grants::into_host`]. There is one way to make an empty one and it is
/// called [`Grants::none`], because "the default" and "nothing at all" being
/// the same thing here is worth saying out loud at every call site.
pub struct Grants {
    console: Option<Sink>,
    input: Option<Vec<String>>,
    clock: bool,
    files: Option<PathBuf>,
    network: Option<deed_rt::Reach>,
    arguments: Option<Vec<String>>,
    environment: Option<Vec<(String, String)>>,
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
        Self {
            console: None,
            input: None,
            clock: false,
            files: None,
            network: None,
            arguments: None,
            environment: None,
        }
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

    /// Grant a directory, and everything under it.
    ///
    /// What `sys.files` narrows to. The rules about what is under it are
    /// `deed_rt::sandbox`'s, which is where the interpreter asks too: a rule
    /// about what a `Dir` reaches that lives inside one of two hosts is a
    /// rule about one of them.
    ///
    /// The path is resolved here, because "under this directory" is a
    /// question about where the directory actually is and a relative path
    /// answers it differently depending on who is asking. A path that does
    /// not resolve grants nothing, so a program that wanted files is turned
    /// down at link time by name rather than told every file is missing.
    pub fn files(mut self, root: PathBuf) -> Self {
        self.files = deed_rt::sandbox::root(&root).ok();
        self
    }

    /// Grant the network, and say which hosts.
    ///
    /// The set is `deed_rt::Reach`, which is where the interpreter asks too.
    /// A `Net` is worth what a `Dir` is worth only if the hosts it reaches
    /// can shrink and never grow, and that rule has to be one rule.
    ///
    /// A `Reach` that grants nothing is still a grant: the program gets the
    /// import and every URL is refused by name, which is a different thing
    /// from the module being turned down at link time for asking.
    pub fn network(mut self, reach: deed_rt::Reach) -> Self {
        self.network = Some(reach);
        self
    }

    /// Grant what somebody typed, a line at a time.
    ///
    /// Reading the console rather than writing to it, which is a separate
    /// entry in the row on the same capability. Running out is `err`, because
    /// an empty line is a real answer and has to stay one.
    pub fn input(mut self, lines: Vec<String>) -> Self {
        self.input = Some(lines);
        self
    }

    /// Grant the arguments the program was invoked with.
    ///
    /// Data rather than authority, which is why it hands back a list. It
    /// still takes the root, so a function that wants them has to have been
    /// handed everything.
    pub fn arguments(mut self, arguments: Vec<String>) -> Self {
        self.arguments = Some(arguments);
        self
    }

    /// Grant these environment variables, and no others.
    ///
    /// By name rather than whole. An environment is whatever the machine
    /// happened to be carrying and it routinely carries credentials, so a
    /// name nobody granted reads as not granted, which is a different fact
    /// from unset and only one of them is about the machine.
    pub fn environment(mut self, variables: Vec<(String, String)>) -> Self {
        self.environment = Some(variables);
        self
    }

    /// Build the host that answers for these grants.
    pub fn into_host(self) -> Granted {
        let mut held = vec![Held::System];
        let has_console = self.console.is_some();
        let has_input = self.input.is_some();
        if has_console {
            held.push(Held::Console);
        }
        if self.clock {
            held.push(Held::Clock);
        }
        let files = self.files;
        if let Some(root) = &files {
            held.push(Held::Dir(root.clone()));
        }
        let network = self.network;
        if let Some(reach) = &network {
            held.push(Held::Net(reach.clone()));
        }

        let table = Rc::new(RefCell::new(Table {
            held,
            console: self.console,
            input: self.input.unwrap_or_default(),
            read_lines: 0,
            ticks: 0,
        }));
        let system = table
            .borrow()
            .handle(&Held::System)
            .expect("the root is always in the table");

        let mut host = Host::new();
        if has_console {
            let narrow = Rc::clone(&table);
            host.offer("deed:sys", "console", move |call| {
                let table = narrow.borrow();
                table.require(&call, Held::System, "sys.console")?;
                Ok(table.handle(&Held::Console))
            });

            let write = Rc::clone(&table);
            host.offer("deed:io", "write", move |call| {
                let mut table = write.borrow_mut();
                table.require(&call, Held::Console, "Io.write")?;
                let line = text_of(&call, 1, "Io.write")?;
                if let Some(console) = table.console.as_mut() {
                    console(&line);
                }
                Ok(None)
            });
        }

        // Reading the console is a grant of its own on the same capability,
        // so a host that prints does not thereby find out what was typed.
        if has_input {
            let line = Rc::clone(&table);
            host.offer("deed:io", "line", move |mut call| {
                let next = {
                    let mut table = line.borrow_mut();
                    table.require(&call, Held::Console, "Io.line")?;
                    let next = table.input.get(table.read_lines).cloned();
                    if next.is_some() {
                        table.read_lines += 1;
                    }
                    next
                };
                match next {
                    Some(text) => {
                        let written = string(&mut call, &text)?;
                        answer(&mut call, true, written)
                    }
                    None => failed(&mut call, "there is no more input"),
                }
            });
        }

        if let Some(arguments) = self.arguments {
            let table = Rc::clone(&table);
            host.offer("deed:io", "args", move |mut call| {
                table.borrow().require(&call, Held::System, "Io.args")?;
                let mut written = Vec::with_capacity(arguments.len());
                for argument in &arguments {
                    written.push(string(&mut call, argument)?);
                }
                call.write_list(&written)
                    .map(Some)
                    .ok_or_else(|| Trap::Refused(NO_ROOM.to_string()))
            });
        }

        if let Some(variables) = self.environment {
            let table = Rc::clone(&table);
            host.offer("deed:io", "env", move |mut call| {
                table.borrow().require(&call, Held::System, "Io.env")?;
                let wanted = text_of(&call, 1, "Io.env")?;
                match variables
                    .iter()
                    .find(|(name, _)| *name == wanted)
                    .map(|(_, value)| value.clone())
                {
                    Some(value) => {
                        let written = string(&mut call, &value)?;
                        answer(&mut call, true, written)
                    }
                    None => failed(
                        &mut call,
                        &format!("`{wanted}` was not granted to this program"),
                    ),
                }
            });
        }

        if self.clock {
            let narrow = Rc::clone(&table);
            host.offer("deed:sys", "clock", move |call| {
                let table = narrow.borrow();
                table.require(&call, Held::System, "sys.clock")?;
                Ok(table.handle(&Held::Clock))
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

        if let Some(root) = files {
            let narrow = Rc::clone(&table);
            host.offer("deed:sys", "files", move |call| {
                let table = narrow.borrow();
                table.require(&call, Held::System, "sys.files")?;
                Ok(table.handle(&Held::Dir(root.clone())))
            });

            let read = Rc::clone(&table);
            host.offer("deed:io", "read", move |mut call| {
                let dir = read.borrow().dir(&call, "Io.read")?;
                let name = text_of(&call, 1, "Io.read")?;
                match deed_rt::sandbox::resolve(&dir, &name) {
                    Ok(path) => match std::fs::read_to_string(&path) {
                        Ok(contents) => {
                            let text = string(&mut call, &contents)?;
                            answer(&mut call, true, text)
                        }
                        Err(error) => failed(&mut call, &format!("`{name}`: {error}")),
                    },
                    Err(refused) => failed(&mut call, &refused.message(&name)),
                }
            });

            let save = Rc::clone(&table);
            host.offer("deed:io", "save", move |mut call| {
                let dir = save.borrow().dir(&call, "Io.save")?;
                let name = text_of(&call, 1, "Io.save")?;
                let contents = text_of(&call, 2, "Io.save")?;
                match deed_rt::sandbox::resolve_new(&dir, &name) {
                    Ok(path) => match std::fs::write(&path, contents) {
                        Ok(()) => answer(&mut call, true, NOTHING),
                        Err(error) => failed(&mut call, &format!("`{name}`: {error}")),
                    },
                    Err(refused) => failed(&mut call, &refused.message(&name)),
                }
            });

            let remove = Rc::clone(&table);
            host.offer("deed:io", "remove", move |mut call| {
                let dir = remove.borrow().dir(&call, "Io.remove")?;
                let name = text_of(&call, 1, "Io.remove")?;
                match deed_rt::sandbox::resolve(&dir, &name) {
                    Ok(path) if path.is_dir() => {
                        failed(&mut call, &format!("`{name}` is a directory"))
                    }
                    Ok(path) => match std::fs::remove_file(&path) {
                        Ok(()) => answer(&mut call, true, NOTHING),
                        Err(error) => failed(&mut call, &format!("`{name}`: {error}")),
                    },
                    Err(refused) => failed(&mut call, &refused.message(&name)),
                }
            });

            let list = Rc::clone(&table);
            host.offer("deed:io", "list", move |mut call| {
                let dir = list.borrow().dir(&call, "Io.list")?;
                match std::fs::read_dir(&dir) {
                    Ok(entries) => {
                        let mut names: Vec<String> = entries
                            .filter_map(|entry| {
                                let entry = entry.ok()?;
                                entry.file_type().ok()?.is_file().then_some(())?;
                                entry.file_name().into_string().ok()
                            })
                            .collect();
                        names.sort();
                        let mut written = Vec::with_capacity(names.len());
                        for name in &names {
                            written.push(string(&mut call, name)?);
                        }
                        let items = call
                            .write_list(&written)
                            .ok_or_else(|| Trap::Refused(NO_ROOM.to_string()))?;
                        answer(&mut call, true, items)
                    }
                    Err(error) => failed(&mut call, &format!("{error}")),
                }
            });

            let open = Rc::clone(&table);
            host.offer("deed:io", "open", move |mut call| {
                let dir = open.borrow().dir(&call, "Io.open")?;
                let name = text_of(&call, 1, "Io.open")?;
                match deed_rt::sandbox::resolve(&dir, &name) {
                    Ok(path) if path.is_dir() => {
                        let handle = open.borrow_mut().hand_out(Held::Dir(path));
                        answer(&mut call, true, handle)
                    }
                    Ok(_) => failed(&mut call, &format!("`{name}` is not a directory")),
                    Err(refused) => failed(&mut call, &refused.message(&name)),
                }
            });

            let make = Rc::clone(&table);
            host.offer("deed:io", "make", move |mut call| {
                let dir = make.borrow().dir(&call, "Io.make")?;
                let name = text_of(&call, 1, "Io.make")?;
                match deed_rt::sandbox::resolve_new(&dir, &name) {
                    Ok(path) if path.exists() => {
                        failed(&mut call, &format!("`{name}` is already there"))
                    }
                    Ok(path) => match std::fs::create_dir(&path) {
                        Ok(()) => match deed_rt::sandbox::root(&path) {
                            Ok(made) => {
                                let handle = make.borrow_mut().hand_out(Held::Dir(made));
                                answer(&mut call, true, handle)
                            }
                            Err(refused) => failed(&mut call, &refused.message(&name)),
                        },
                        Err(error) => failed(&mut call, &format!("`{name}`: {error}")),
                    },
                    Err(refused) => failed(&mut call, &refused.message(&name)),
                }
            });
        }

        if let Some(reach) = network {
            let narrow = Rc::clone(&table);
            host.offer("deed:sys", "net", move |call| {
                let table = narrow.borrow();
                table.require(&call, Held::System, "sys.net")?;
                Ok(table.handle(&Held::Net(reach.clone())))
            });

            let widen = Rc::clone(&table);
            host.offer("deed:io", "reach", move |mut call| {
                let reach = widen.borrow().net(&call, "Io.reach")?;
                let host = text_of(&call, 1, "Io.reach")?;
                match deed_rt::reach::narrow(&reach, &host) {
                    Ok(narrowed) => {
                        let handle = widen.borrow_mut().hand_out(Held::Net(narrowed));
                        answer(&mut call, true, handle)
                    }
                    Err(refused) => failed(&mut call, &refused.message(&host)),
                }
            });

            let fetch = Rc::clone(&table);
            host.offer("deed:io", "fetch", move |mut call| {
                let reach = fetch.borrow().net(&call, "Io.fetch")?;
                let url = text_of(&call, 1, "Io.fetch")?;
                asked(
                    &mut call,
                    deed_rt::over_the_network(&reach, &url, "GET", None),
                )
            });

            let send = Rc::clone(&table);
            host.offer("deed:io", "send", move |mut call| {
                let reach = send.borrow().net(&call, "Io.send")?;
                let url = text_of(&call, 1, "Io.send")?;
                let body = text_of(&call, 2, "Io.send")?;
                asked(
                    &mut call,
                    deed_rt::over_the_network(&reach, &url, "POST", Some(&body)),
                )
            });
        }

        Granted { host, system }
    }
}

/// What a `Result<(), String>` carries in its `ok`.
///
/// A word, because every field is one. Nothing reads it: a field with no
/// representation is dropped where it is read, which is what `ok(nothing)`
/// binds.
const NOTHING: Value = Value::I64(0);

/// What a host says when the module has nowhere to put the answer.
const NO_ROOM: &str = "the module has no memory left for the answer";

/// The string argument at this position.
///
/// A refusal rather than an `err`, because a module that passes something
/// that is not a string where the import declares one is not a program
/// having a bad day, it is a module that does not match its own signature.
fn text_of(call: &HostCall<'_>, at: usize, operation: &str) -> Result<String, Trap> {
    call.text(at).ok_or_else(|| {
        Trap::Refused(format!(
            "`{operation}` was handed something that is not a string"
        ))
    })
}

/// A string written into the module's memory.
fn string(call: &mut HostCall<'_>, text: &str) -> Result<Value, Trap> {
    call.write_text(text)
        .ok_or_else(|| Trap::Refused(NO_ROOM.to_string()))
}

/// A `Result` written into the module's memory.
///
/// Which tag is which comes from `deed_mir`, which is where the layout is
/// synthesized. A second copy of that order here would be an answer that is
/// inverted rather than wrong, which is the kind nobody notices.
fn answer(call: &mut HostCall<'_>, ok: bool, value: Value) -> Result<Option<Value>, Trap> {
    let tag = deed_mir::result_variant(if ok { "ok" } else { "err" })
        .expect("`ok` and `err` are what a Result is made of");
    call.write_aggregate(Some(tag), &[value])
        .map(Some)
        .ok_or_else(|| Trap::Refused(NO_ROOM.to_string()))
}

/// An `err` carrying a sentence.
fn failed(call: &mut HostCall<'_>, why: &str) -> Result<Option<Value>, Trap> {
    let text = string(call, why)?;
    answer(call, false, text)
}

/// A `Result<String, String>` some other part of the runtime already decided.
fn asked(call: &mut HostCall<'_>, what: Result<String, String>) -> Result<Option<Value>, Trap> {
    match what {
        Ok(text) => {
            let written = string(call, &text)?;
            answer(call, true, written)
        }
        Err(why) => failed(call, &why),
    }
}

/// The handles this host has handed out, and what they stand for.
struct Table {
    held: Vec<Held>,
    console: Option<Sink>,
    input: Vec<String>,
    read_lines: usize,
    ticks: i64,
}

impl Table {
    /// The handle for this capability, when it was granted.
    ///
    /// One past the index, so that zero is never a handle. A module that
    /// passes an uninitialised word gets a refusal rather than the console.
    fn handle(&self, what: &Held) -> Option<Value> {
        self.held
            .iter()
            .position(|one| one == what)
            .map(|at| Value::I64(at as i64 + 1))
    }

    /// The handle for a directory, adding it to the table the first time.
    ///
    /// Interned by resolved path rather than one entry per `Io.open`, so a
    /// program that opens the same place in a loop does not grow the table
    /// for as long as it runs. Two handles to one directory would reach the
    /// same things anyway, so there is nothing to tell apart.
    fn hand_out(&mut self, what: Held) -> Value {
        if let Some(already) = self.handle(&what) {
            return already;
        }
        self.held.push(what);
        Value::I64(self.held.len() as i64)
    }

    /// What a handle the module passed back stands for, if anything.
    fn stands_for(&self, value: Option<Value>) -> Option<&Held> {
        let index = usize::try_from(value?.as_i64().checked_sub(1)?).ok()?;
        self.held.get(index)
    }

    /// Check that the capability argument is the one this operation acts on.
    fn require(&self, call: &HostCall<'_>, what: Held, operation: &str) -> Result<(), Trap> {
        if self
            .stands_for(call.arg(0))
            .is_some_and(|held| held.is(&what))
        {
            return Ok(());
        }
        Err(self.refusal(&what, operation))
    }

    /// The directory the capability argument stands for.
    fn dir(&self, call: &HostCall<'_>, operation: &str) -> Result<PathBuf, Trap> {
        match self.stands_for(call.arg(0)) {
            Some(Held::Dir(path)) => Ok(path.clone()),
            _ => Err(self.refusal(&Held::Dir(PathBuf::new()), operation)),
        }
    }

    /// The hosts the capability argument reaches.
    fn net(&self, call: &HostCall<'_>, operation: &str) -> Result<deed_rt::Reach, Trap> {
        match self.stands_for(call.arg(0)) {
            Some(Held::Net(reach)) => Ok(reach.clone()),
            _ => Err(self.refusal(&Held::Net(deed_rt::Reach::none()), operation)),
        }
    }

    /// What this host says when it was handed something it did not grant.
    fn refusal(&self, want: &Held, operation: &str) -> Trap {
        Trap::Refused(format!(
            "`{operation}` was handed something that is not a `{}` this host granted",
            want.name()
        ))
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

    /// Read a Deed string out of a `Result` the host wrote.
    fn said(memory: &mut [u8], answer: Option<Value>) -> String {
        let at = answer.expect("it answers with something").as_i64() as usize;
        let field = i64::from_le_bytes(memory[at + 8..at + 16].try_into().expect("a word"));
        let held = [Value::I64(field)];
        HostCall::new(&held, memory)
            .text(0)
            .expect("the field is a string")
    }

    /// Which variant a `Result` the host wrote carries.
    fn outcome(memory: &[u8], answer: Option<Value>) -> i64 {
        let at = answer.expect("it answers with something").as_i64() as usize;
        i64::from_le_bytes(memory[at..at + 8].try_into().expect("a word"))
    }

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
        let Trap::Refused(why) = stopped else {
            panic!("it should be refused rather than answered");
        };
        assert!(
            why.contains("`Console`"),
            "it should say what it wanted: {why}"
        );
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

    /// And `Io.epoch` reads the machine, which is the whole difference
    /// between the two: one of them can give the same answer twice.
    #[test]
    fn the_wall_clock_reads_the_machine() {
        let granted = Grants::none().clock().into_host();
        let mut memory = vec![0u8; 64];
        let clock = granted
            .host
            .implementation_for("deed:sys", "clock")
            .expect("a clock was granted")(HostCall::new(
            &[granted.system],
            &mut memory,
        ))
        .expect("narrowing the root is allowed")
        .expect("it answers with a handle");

        let answer =
            granted
                .host
                .implementation_for("deed:io", "epoch")
                .expect("a clock was granted")(HostCall::new(&[clock], &mut memory))
            .expect("granted")
            .expect("it answers with a number")
            .as_i64();

        // Some time after this was written, and not a tick count.
        assert!(
            answer > 1_750_000_000_000,
            "the wall clock should read the machine, not count: {answer}"
        );
    }

    /// Reading the console hands over one line at a time, and says when
    /// there are no more.
    ///
    /// Running out is `err` rather than an empty line, because a program
    /// that cannot tell "somebody typed nothing" from "there is nothing
    /// left" either loops forever or stops early.
    #[test]
    fn reading_the_console_hands_over_one_line_at_a_time() {
        let granted = Grants::none()
            .console(|_| {})
            .input(vec!["first".to_string(), String::new()])
            .into_host();
        let mut memory = vec![0u8; 512];
        memory[..8].copy_from_slice(&64u64.to_le_bytes());

        let console = granted
            .host
            .implementation_for("deed:sys", "console")
            .expect("a console was granted")(HostCall::new(
            &[granted.system],
            &mut memory,
        ))
        .expect("narrowing the root is allowed")
        .expect("it answers with a handle");

        let line = granted
            .host
            .implementation_for("deed:io", "line")
            .expect("input was granted");

        let first = line(HostCall::new(&[console], &mut memory)).expect("granted");
        assert_eq!(outcome(&memory, first), 0, "an ok");
        assert_eq!(said(&mut memory, first), "first");

        let second = line(HostCall::new(&[console], &mut memory)).expect("granted");
        assert_eq!(outcome(&memory, second), 0, "an empty line is an answer");
        assert_eq!(said(&mut memory, second), "");

        let third = line(HostCall::new(&[console], &mut memory)).expect("granted");
        assert_eq!(outcome(&memory, third), 1, "an err");
        assert_eq!(said(&mut memory, third), "there is no more input");
    }

    /// A console that prints does not thereby find out what was typed.
    #[test]
    fn granting_a_console_does_not_grant_reading_it() {
        let granted = Grants::none().console(|_| {}).into_host();
        assert!(granted.host.offers("deed:io", "write"));
        assert!(!granted.host.offers("deed:io", "line"));
    }

    /// One variable, and only the ones this run was told to hand over.
    ///
    /// A name nobody granted reads as not granted rather than as unset,
    /// because those are different facts and only one of them is about the
    /// machine.
    #[test]
    fn only_the_granted_environment_variables_are_there() {
        let granted = Grants::none()
            .environment(vec![("GRANTED".to_string(), "yes".to_string())])
            .into_host();
        let mut memory = vec![0u8; 512];
        memory[..8].copy_from_slice(&64u64.to_le_bytes());

        let env = granted
            .host
            .implementation_for("deed:io", "env")
            .expect("the environment was granted");

        let wanted = {
            let nothing: [Value; 0] = [];
            HostCall::new(&nothing, &mut memory)
                .write_text("GRANTED")
                .expect("there is room")
        };
        let found = env(HostCall::new(&[granted.system, wanted], &mut memory)).expect("granted");
        assert_eq!(outcome(&memory, found), 0, "an ok");
        assert_eq!(said(&mut memory, found), "yes");

        let other = {
            let nothing: [Value; 0] = [];
            HostCall::new(&nothing, &mut memory)
                .write_text("SECRET")
                .expect("there is room")
        };
        let missing = env(HostCall::new(&[granted.system, other], &mut memory)).expect("granted");
        assert_eq!(outcome(&memory, missing), 1, "an err");
        assert_eq!(
            said(&mut memory, missing),
            "`SECRET` was not granted to this program"
        );
    }
}
