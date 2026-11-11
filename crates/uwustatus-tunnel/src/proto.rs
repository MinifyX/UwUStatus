//! The messages on the control stream: each one is JSON behind its length as a big-endian `u32`.
//!
//! Main instance → satellite: [`MainMessage`]; satellite → main instance: [`SatelliteMessage`].
//! What a check is and what it found stays opaque JSON here, so this crate needs no probe types.

use std::io;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The version in [`MainMessage::Hello`]; a satellite refuses any other.
pub const VERSION: u32 = 1;
/// The largest message either side sends or reads.
pub const MAX_MESSAGE: usize = 1024 * 1024;
/// The largest message the satellite reads before it knows who is calling.
pub(crate) const MAX_HELLO: usize = 4 * 1024;

/// One check the satellite runs on its own schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: String,
    pub interval_s: u32,
    pub check: Value,
}

/// What one run of a check found. `seq` grows with every result of a satellite, across restarts
/// too, so the main instance can store results once per `(satellite, seq)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultItem {
    pub seq: u64,
    pub assignment_id: String,
    pub at_unix_ms: i64,
    pub outcome: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MainMessage {
    /// The first message. `token` is the one from the pairing code, until the satellite knows the
    /// main instance.
    Hello {
        version: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// The complete list; it replaces the one before.
    Assign { checks: Vec<Assignment> },
    /// The dead man's switch: deadline, alert channels, emergency status page. Opaque here.
    DeadMan { config: Value },
    /// Results up to and including this `seq` are stored; the satellite forgets them.
    Ack { up_to_seq: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SatelliteMessage {
    Welcome { version: u32, satellite_version: String },
    Refused { reason: Refusal, message: String },
    Results { items: Vec<ResultItem> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The main instance speaks another tunnel version.
    Version,
    /// The satellite is not paired and the hello brings no token.
    NotPaired,
    /// The satellite is paired with another main instance.
    OtherMain,
    /// The token is wrong or used up.
    WrongToken,
    /// A reason from a newer satellite.
    #[serde(other)]
    Other,
}

pub async fn write_message<W, T>(writer: &mut W, message: &T) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(message).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "tunnel message too big"));
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    writer.write_all(&frame).await?;
    writer.flush().await
}

/// Reads exactly one message of at most `max` bytes. The length is checked before anything is
/// allocated for the body.
pub async fn read_message<R, T>(reader: &mut R, max: usize) -> io::Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    if length > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "tunnel message too big"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// Like [`read_message`], but a well-formed message this version does not know comes back as
/// `None`, so a newer other side never has to break off the control stream to stay understood.
pub async fn read_known<R, T>(reader: &mut R, max: usize) -> io::Result<Option<T>>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let value: Value = read_message(reader, max).await?;
    Ok(serde_json::from_value(value).ok())
}
