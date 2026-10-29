//! The relay's WebSocket, from either end: a host's link, or a device's way to a host. See
//! [`remote_proto::relay`].
//!
//! [`Client::open_relay`] asks for a ticket with the access credential, opens the socket,
//! presents the ticket and reads the relay's greeting. What it hands back is a [`LinkReader`] and
//! a [`LinkWriter`], to be used from separate tasks: reading never waits on writing.
//!
//! The access credential goes only to the HTTPS ticket request. The socket is `wss://`, checked
//! against the operating system's roots as every other request is, except to a loopback address
//! written as one, as [`Client::new`] allows.

use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use remote_proto::account::Secret;
use remote_proto::relay::{
    CloseReason, Greeting, Presented, RelayTicket, TicketRequest, MAX_MESSAGE, ROUTE_PARAM,
    SILENCE_MS,
};
use remote_proto::HostId;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::{chain, Client, ClientError, Result};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How long opening the socket and hearing the relay's greeting may take.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// A connected relay socket, as its two halves.
pub struct RelayLink {
    /// The host this socket reaches, or is.
    pub host: HostId,
    pub reader: LinkReader,
    pub writer: LinkWriter,
}

/// What arrived on a relay socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Text(String),
    Binary(Vec<u8>),
}

pub struct LinkReader(SplitStream<Socket>);

impl LinkReader {
    /// The next message, or why the socket closed. Pings are answered and not shown. A socket
    /// that says nothing at all for [`SILENCE_MS`] is taken to be dead.
    pub async fn next(&mut self) -> std::result::Result<Incoming, CloseReason> {
        let silence = Duration::from_millis(SILENCE_MS);
        loop {
            match tokio::time::timeout(silence, self.0.next()).await {
                Err(_) | Ok(None) | Ok(Some(Err(_))) => return Err(CloseReason::Gone),
                Ok(Some(Ok(Message::Text(text)))) => return Ok(Incoming::Text(text.to_string())),
                Ok(Some(Ok(Message::Binary(bytes)))) => return Ok(Incoming::Binary(bytes.into())),
                Ok(Some(Ok(Message::Close(frame)))) => {
                    return Err(frame.map_or(CloseReason::Gone, |f| {
                        CloseReason::from_code(u16::from(f.code))
                    }))
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }
}

pub struct LinkWriter(SplitSink<Socket, Message>);

impl LinkWriter {
    pub async fn text(&mut self, text: String) -> Result<()> {
        self.send(Message::Text(text.into())).await
    }

    pub async fn binary(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.send(Message::Binary(bytes.into())).await
    }

    /// Say goodbye and close the socket.
    pub async fn close(&mut self) {
        let _ = self
            .0
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            })))
            .await;
        let _ = self.0.close().await;
    }

    async fn send(&mut self, message: Message) -> Result<()> {
        self.0
            .send(message)
            .await
            .map_err(|e| ClientError::Unreachable(chain(&e)))
    }
}

impl Client {
    /// `POST /v1/relay/tickets`.
    pub async fn ticket(&self, access: &Secret, request: &TicketRequest) -> Result<RelayTicket> {
        let url = self.url("v1/relay/tickets")?;
        self.send(self.authed(self.http.post(url), access).json(request))
            .await
    }

    /// Ask for a ticket and open the relay with it: a host's own link with
    /// [`TicketRequest::Host`], or a device's way to a host with [`TicketRequest::Device`].
    pub async fn open_relay(&self, access: &Secret, request: &TicketRequest) -> Result<RelayLink> {
        let ticket = self.ticket(access, request).await?;
        self.open_relay_with(ticket).await
    }

    /// Open the relay with a ticket already in hand, at the instance its route names.
    pub async fn open_relay_with(&self, ticket: RelayTicket) -> Result<RelayLink> {
        tokio::time::timeout(OPEN_TIMEOUT, self.present(ticket))
            .await
            .map_err(|_| ClientError::Unreachable("the relay did not answer in time".into()))?
    }

    async fn present(&self, RelayTicket { ticket, route, .. }: RelayTicket) -> Result<RelayLink> {
        let mut url = self.url("v1/relay")?;
        if let Some(route) = route {
            url.query_pairs_mut().append_pair(ROUTE_PARAM, &route);
        }
        let tls = url.scheme() == "https";
        url.set_scheme(if tls { "wss" } else { "ws" })
            .map_err(|()| ClientError::Invalid("not a relay address".into()))?;
        let connector = if tls {
            use rustls_platform_verifier::ConfigVerifierExt;
            let config = rustls::ClientConfig::with_platform_verifier()
                .map_err(|e| ClientError::Invalid(e.to_string()))?;
            Connector::Rustls(Arc::new(config))
        } else {
            Connector::Plain
        };
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(
            url.as_str(),
            Some(config),
            true,
            Some(connector),
        )
        .await
        .map_err(|e| ClientError::Unreachable(chain(&e)))?;
        let (writer, reader) = socket.split();
        let (mut reader, mut writer) = (LinkReader(reader), LinkWriter(writer));
        let presented = serde_json::to_string(&Presented { ticket }).expect("serialises");
        writer.text(presented).await?;
        let greeting = match reader.next().await {
            Ok(Incoming::Text(text)) => serde_json::from_str::<Greeting>(&text)
                .map_err(|e| ClientError::Unexpected(e.to_string()))?,
            Ok(Incoming::Binary(_)) => {
                return Err(ClientError::Unexpected("binary before the greeting".into()))
            }
            Err(reason) => return Err(ClientError::Unreachable(reason.text().into())),
        };
        match greeting {
            Greeting::Connected { host } => Ok(RelayLink {
                host,
                reader,
                writer,
            }),
            Greeting::Refused(error) => Err(ClientError::Remote(error)),
        }
    }
}
