use std::collections::{hash_map::Entry, HashMap};

use http::{uri::Scheme, Request};
use mio::{Events, Poll, Registry};
use mio_httpc::{Call, CallBuilder, CallRef, Httpc, RecvState, Response, SendState};
use thiserror::Error;

use crate::utils::full_last_part_of_typename;

#[allow(dead_code)]
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RpcResponse<T> {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<T>,
}

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
#[repr(u8)]
pub enum Error {
    #[error("mio_httpc call creation error")]
    CallCreation,
    #[error("http call timeout")]
    Timeout(CallRef),
    #[error("mio_httpc went into a wrong state while sending: Done")]
    DoneWhileSending(CallRef),
    #[error("mio_httpc returned SendState::Error: {1}")]
    Send(CallRef, mio_httpc::Error),
    #[error("mio_httpc returned RecvState::Error: {1}")]
    Recv(CallRef, mio_httpc::Error),
    #[error("mio_httpc returned RecvState::Done which should never happen before full body is returned")]
    DoneWhileReceiving(CallRef),
}
impl Error {
    pub fn callref(&self) -> &CallRef {
        match self {
            Error::CallCreation => unreachable!(),
            Error::Timeout(call_ref) => call_ref,
            Error::DoneWhileSending(call_ref) => call_ref,
            Error::Send(call_ref, _) => call_ref,
            Error::Recv(call_ref, _) => call_ref,
            Error::DoneWhileReceiving(call_ref) => call_ref,
        }
    }
}

pub struct Connection {
    /// ingstion time of the message that caused this request
    call: Call,
    receiving: bool,
    response: Response,
}

impl Connection {
    fn new(call: Call) -> Self {
        Self { call, receiving: false, response: Default::default() }
    }

    fn drive_send(&mut self, httpc: &mut Httpc, registry: &Registry) -> Result<()> {
        loop {
            // None because we are not sending any body as that was created during the CallBuilder step
            match httpc.call_send(registry, &mut self.call, None) {
                SendState::Done => return Err(Error::DoneWhileSending(self.call.get_ref())),
                SendState::Error(e) => return Err(Error::Send(self.call.get_ref(), e)),
                SendState::Wait => return Ok(()),
                SendState::SentBody(_) => {}
                SendState::Receiving => {
                    self.receiving = true;
                    return Ok(());
                }
                e => {
                    panic!("at {e:?}")
                }
            }
        }
    }

    fn drive_recv(&mut self, httpc: &mut Httpc, registry: &Registry) -> Option<Result<(CallRef, Response, Vec<u8>)>> {
        loop {
            match httpc.call_recv(registry, &mut self.call, None) {
                RecvState::Done => {
                    return Some(Err(Error::DoneWhileReceiving(self.call.get_ref())));
                }
                RecvState::Response(resp, _) => {
                    self.response = resp;
                }
                RecvState::ReceivedBody(_) => {}
                RecvState::DoneWithBody(rt) => {
                    return Some(Ok((self.call.get_ref(), std::mem::take(&mut self.response), rt)));
                }
                RecvState::Error(e) => {
                    return Some(Err(Error::Recv(self.call.get_ref(), e)));
                }
                RecvState::Sending => {
                    self.receiving = false;
                    return None;
                }
                RecvState::Wait => {
                    tracing::debug!("waiting for {:?}", self.call.get_ref());
                    return None;
                }
            }
        }
    }

    fn drive(&mut self, httpc: &mut Httpc, registry: &Registry) -> Option<Result<(CallRef, Response, Vec<u8>)>> {
        if !self.receiving {
            if let Err(e) = self.drive_send(httpc, registry) {
                return Some(Err(e));
            }
        }
        if self.receiving {
            return self.drive_recv(httpc, registry);
        }
        None
    }
}

pub struct WalkieTalkie {
    httpc: Httpc,
    poll: Poll,
    events: Events,
    requests: HashMap<CallRef, Connection>,
}

impl Default for WalkieTalkie {
    fn default() -> Self {
        Self {
            httpc: Httpc::new(10, None),
            poll: Poll::new()
                .inspect_err(|e| {
                    panic!("Couldn't get a poll for the http client {}: {e}", full_last_part_of_typename::<Self>())
                })
                .unwrap(),
            events: Events::with_capacity(128),
            requests: HashMap::default(),
        }
    }
}

impl WalkieTalkie {
    pub fn new(start_token: usize) -> Self {
        Self { httpc: Httpc::new(start_token, None), ..Default::default() }
    }

    pub fn send(&mut self, req: Request<Vec<u8>>) -> Result<CallRef> {
        let mut call_builder = CallBuilder::new();
        call_builder
            .method(req.method().as_str())
            .url(&format!("{}", req.uri()))
            .map_err(|e| {
                tracing::error!("issue creating call: {e}");
                Error::CallCreation
            })?
            .set_https(req.uri().scheme() == Some(&Scheme::HTTPS))
            .chunked_parse(true)
            .chunked_max_chunk(1024 * 1024)
            .timeout_ms(2000);

        for (name, value) in req.headers() {
            if let Ok(value_str) = value.to_str() {
                call_builder.header(name.as_str(), value_str);
            }
        }

        call_builder.body(req.into_body());

        let call = call_builder.call(&mut self.httpc, self.poll.registry()).map_err(|e| {
            tracing::error!("issue creating call: {e}");
            Error::CallCreation
        })?;

        let r = call.get_ref();
        let connection = Connection::new(call);
        self.requests.insert(r, connection);
        Ok(r)
    }

    pub fn send_blocking(&mut self, req: Request<Vec<u8>>) -> Result<(Response, Vec<u8>)> {
        self.send(req)?;
        let mut responses = Vec::new();
        while responses.len() != 1 {
            self.gather_responses(&mut responses);
        }
        responses.drain(..).next().ok_or(Error::CallCreation)?.map(|(_, response, body)| (response, body))
    }

    fn poll(&mut self) -> bool {
        self.poll
            .poll(&mut self.events, Some(std::time::Duration::from_nanos(0)))
            .inspect_err(|e| tracing::error!("Couldn't poll for updates on http client: {e}"))
            .is_ok()
    }

    pub fn gather_responses(&mut self, v: &mut Vec<Result<(CallRef, Response, Vec<u8>)>>) {
        if !self.poll() {
            return;
        }
        for r in self.httpc.timeout() {
            let Some(req) = self.requests.remove(&r) else {
                continue;
            };
            self.httpc.call_close(req.call);
            v.push(Err(Error::Timeout(r)));
        }

        for ev in self.events.iter() {
            let Some(call) = self.httpc.event(ev) else {
                continue;
            };
            let Entry::Occupied(mut connection) = self.requests.entry(call) else {
                continue;
            };

            if let Some(res) = connection.get_mut().drive(&mut self.httpc, self.poll.registry()) {
                let (_r, connection) = connection.remove_entry();
                if res.is_err() {
                    self.httpc.call_close(connection.call);
                }
                v.push(res)
            }
        }
    }
}
