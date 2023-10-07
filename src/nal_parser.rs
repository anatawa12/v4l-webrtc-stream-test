use std::fmt::Formatter;

pub struct H264Parser<'a> {
    buffer: &'a [u8],
}

impl<'a> H264Parser<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer }
    }

    pub fn next_buffer(&mut self) -> Result<Option<&'a [u8]>, H264ParserError> {
        if self.buffer.len() == 0 {
            return Ok(None);
        }

        // 0x00_00_01 or 0x00_00_00_01
        if self.buffer.len() < 3 || self.buffer[0] != 0 || self.buffer[1] != 0 {
            return Err(H264ParserError::InvalidHeader);
        }
        match self.buffer[2] {
            1 => self.buffer = &self.buffer[3..],
            0 => {
                if self.buffer.len() < 4 || self.buffer[3] != 1 {
                    return Err(H264ParserError::InvalidHeader);
                }
                self.buffer = &self.buffer[4..]
            }
            _ => return Err(H264ParserError::InvalidHeader),
        }

        //let mut
        let mut index = 0;
        let mut zero_count = 0;
        while index < self.buffer.len() {
            match self.buffer[index] {
                0 => zero_count += 1,
                1 if zero_count >= 2 => {
                    let header = if zero_count == 2 { 2 } else { 3 };

                    let (nal, rest) = self.buffer.split_at(index - header);
                    self.buffer = rest;
                    return Ok(Some(nal));
                }
                _ => zero_count = 0,
            }
            index += 1
        }

        Ok(Some(std::mem::replace(&mut self.buffer, &[])))
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum H264ParserError {
    InvalidHeader,
}

impl std::error::Error for H264ParserError {}

impl std::fmt::Display for H264ParserError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            H264ParserError::InvalidHeader => f.write_str("Invalid NAL Header"),
        }
    }
}
