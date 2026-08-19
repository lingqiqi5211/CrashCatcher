use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};

use cch_wire::{LENGTH_PREFIX_BYTES, decode_length, encode_frame};

pub fn read_json_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut prefix = [0_u8; LENGTH_PREFIX_BYTES];
    reader.read_exact(&mut prefix)?;
    let length = decode_length(&prefix).map_err(invalid_data)?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(invalid_data)
}

pub fn write_json_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(invalid_data)?;
    let frame = encode_frame(&body).map_err(invalid_data)?;
    writer.write_all(&frame)
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cch_wire::ChannelKind;

    #[test]
    fn json_frames_round_trip() {
        let mut bytes = Vec::new();
        write_json_frame(&mut bytes, &ChannelKind::Control).expect("writes");
        let parsed = read_json_frame::<_, ChannelKind>(&mut bytes.as_slice()).expect("reads");
        assert_eq!(parsed, ChannelKind::Control);
    }
}
