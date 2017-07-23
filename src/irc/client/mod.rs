use self::action::Action;
pub use self::msg_ctx::MessageContext;
pub use self::reaction::Reaction;
use self::session::Session;
use irc::Error;
use irc::ErrorKind;
use irc::Message;
use irc::Result;
use irc::connection::Connection;
use irc::connection::GenericConnection;
use irc::connection::GetMioTcpStream;
use irc::connection::ReceiveMessage;
use irc::connection::SendMessage;
use mio;
use pircolate;
use std;
use std::io;
use std::io::Write;
use std::sync::mpsc;

pub mod msg_ctx;
pub mod reaction;
pub mod session;

pub mod prelude {
    pub use super::session;
    pub use super::super::Message as IrcMessage;
    pub use super::super::connection::prelude::*;
}

mod action;

#[derive(Debug)]
pub struct Client {
    // TODO: use smallvec.
    sessions: Vec<SessionEntry>,
    mpsc_receiver: mpsc::Receiver<Action>,
    mpsc_registration: mio::Registration,
    handle_prototype: ClientHandle,
}

#[derive(Clone, Debug)]
pub struct ClientHandle {
    mpsc_sender: mpsc::SyncSender<Action>,
    readiness_setter: mio::SetReadiness,
}

#[derive(Debug)]
struct SessionEntry {
    inner: Session<GenericConnection>,
    // TODO: use smallvec.
    output_queue: Vec<Message>,
    is_writable: bool,
}

#[derive(Clone, Debug)]
pub struct SessionId {
    index: usize,
}

const MPSC_QUEUE_SIZE_LIMIT: usize = 1024;
const MPSC_QUEUE_TOKEN: usize = std::usize::MAX;

impl Client {
    pub fn new() -> Self {
        let sessions = Vec::new();
        let (mpsc_sender, mpsc_receiver) = mpsc::sync_channel(MPSC_QUEUE_SIZE_LIMIT);
        let (mpsc_registration, readiness_setter) = mio::Registration::new2();
        let handle_prototype = ClientHandle {
            mpsc_sender,
            readiness_setter,
        };

        Client {
            sessions,
            mpsc_receiver,
            mpsc_registration,
            handle_prototype,
        }
    }

    pub fn handle(&self) -> ClientHandle {
        self.handle_prototype.clone()
    }

    pub fn add_session<Conn>(&mut self, session: Session<Conn>) -> Result<SessionId>
    where
        Conn: Connection,
    {
        let index = self.sessions.len();

        if index == std::usize::MAX {
            // `usize::MAX` is used as the `mio::Token` value for the `Client`'s MPSC queue, and
            // would mean that the upcoming `Vec::push` call would cause an overflow, assuming the
            // system had somehow not run out of memory.

            // TODO: return an error.
            unreachable!()
        }

        self.sessions.push(SessionEntry {
            inner: session.into_generic(),
            output_queue: Vec::new(),
            is_writable: false,
        });

        Ok(SessionId { index: index })
    }

    pub fn run<MsgHandler>(mut self, msg_handler: MsgHandler) -> Result<()>
    where
        MsgHandler: Fn(&MessageContext, Result<Message>) -> Reaction,
    {
        let poll = match mio::Poll::new() {
            Ok(p) => p,
            Err(err) => {
                error!("Failed to construct `mio::Poll`: {} ({:?})", err, err);
                bail!(err)
            }
        };

        let mut events = mio::Events::with_capacity(512);

        for (index, session) in self.sessions.iter().enumerate() {
            poll.register(
                session.inner.mio_tcp_stream(),
                mio::Token(index),
                mio::Ready::readable() | mio::Ready::writable(),
                mio::PollOpt::edge(),
            )?
        }

        poll.register(
            &self.mpsc_registration,
            mio::Token(MPSC_QUEUE_TOKEN),
            mio::Ready::readable(),
            mio::PollOpt::edge(),
        )?;

        loop {
            let _event_qty = poll.poll(&mut events, None)?;

            for event in &events {
                match event.token() {
                    mio::Token(MPSC_QUEUE_TOKEN) => process_mpsc_queue(&mut self),
                    mio::Token(session_index) => {
                        let ref mut session = self.sessions[session_index];
                        process_session_event(
                            event.readiness(),
                            session,
                            session_index,
                            &msg_handler,
                        )
                    }
                }
            }
        }

        Ok(())
    }
}

fn process_session_event<MsgHandler>(
    readiness: mio::Ready,
    session: &mut SessionEntry,
    session_index: usize,
    msg_handler: MsgHandler,
) where
    MsgHandler: Fn(&MessageContext, Result<Message>) -> Reaction,
{
    if readiness.is_writable() {
        session.is_writable = true;
    }

    if session.is_writable {
        process_writable(session, session_index);
    }

    if readiness.is_readable() {
        process_readable(session, session_index, &msg_handler);
    }
}

fn process_readable<MsgHandler>(
    session: &mut SessionEntry,
    session_index: usize,
    msg_handler: MsgHandler,
) where
    MsgHandler: Fn(&MessageContext, Result<Message>) -> Reaction,
{
    let msg_ctx = MessageContext { session_id: SessionId { index: session_index } };
    let msg_handler_with_ctx = move |m| msg_handler(&msg_ctx, m);

    loop {
        let reaction = match session.inner.recv() {
            Ok(Some(ref msg)) if msg.raw_command() == "PING" => {
                match msg.raw_message().replacen("I", "O", 1).parse() {
                    Ok(pong) => Reaction::RawMsg(pong),
                    Err(err) => msg_handler_with_ctx(Err(err.into())),
                }
            }
            Ok(Some(msg)) => msg_handler_with_ctx(Ok(msg)),
            Ok(None) => break,
            Err(Error(ErrorKind::Io(ref err), _))
                if [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut].contains(&err.kind()) => {
                break
            }
            Err(err) => msg_handler_with_ctx(Err(err)),
        };

        process_reaction(session, session_index, reaction);
    }
}

fn process_writable(session: &mut SessionEntry, session_index: usize) {
    let mut msgs_consumed = 0;

    for (index, msg) in session.output_queue.iter().enumerate() {
        match session.inner.try_send(msg.clone()) {
            Ok(()) => msgs_consumed += 1,
            Err(Error(ErrorKind::Io(ref err), _))
                if [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut].contains(&err.kind()) => {
                session.is_writable = false;
                break;
            }
            Err(err) => {
                msgs_consumed += 1;
                error!(
                    "[session {}] Failed to send message {:?} (error: {})",
                    session_index,
                    msg.raw_message(),
                    err
                )
            }
        }
    }

    session.output_queue.drain(..msgs_consumed);
}

fn process_reaction(session: &mut SessionEntry, session_index: usize, reaction: Reaction) {
    match reaction {
        Reaction::None => {}
        Reaction::RawMsg(msg) => session.send(session_index, msg),
        Reaction::Multi(reactions) => {
            for r in reactions {
                process_reaction(session, session_index, r);
            }
        }
    }
}

fn process_mpsc_queue(client: &mut Client) {
    while let Ok(action) = client.mpsc_receiver.try_recv() {
        process_action(client, action)
    }
}

fn process_action(client: &mut Client, action: Action) {
    match action {
        Action::None => {}
        Action::RawMsg {
            session: SessionId { index: session_index },
            message,
        } => {
            let ref mut session = client.sessions[session_index];
            session.send(session_index, message)
        }
    }
}

impl ClientHandle {
    pub fn try_send(&mut self, session: SessionId, message: Message) -> Result<()> {
        // Add the action to the client's MPSC queue.
        self.mpsc_sender
            .try_send(Action::RawMsg { session, message })
            .unwrap();

        // Notify the client that there's an action to read from the MPSC queue.
        self.readiness_setter.set_readiness(mio::Ready::readable())?;

        Ok(())
    }
}

impl SessionEntry {
    fn send(&mut self, session_index: usize, msg: Message) {
        match self.inner.try_send(msg.clone()) {
            Ok(()) => {
                // TODO: log the `session_index`.
            }
            Err(Error(ErrorKind::Io(ref err), _))
                if [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut].contains(&err.kind()) => {
                trace!(
                    "[session {}] Write would block or timed out; enqueueing message for later \
                     transmission: {:?}",
                    session_index,
                    msg.raw_message()
                );
                self.is_writable = false;
                self.output_queue.push(msg);
            }
            Err(err) => {
                error!(
                    "[session {}] Failed to send message {:?} (error: {})",
                    session_index,
                    msg.raw_message(),
                    err
                )
            }
        }
    }
}