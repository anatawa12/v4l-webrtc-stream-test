pub struct H264Parser<'a> {
    buffer: &'a [u8],
}

impl<'a> H264Parser<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer }
    }

    pub fn next_buffer(&mut self) -> Option<&'a [u8]> {
        if self.buffer.len() == 0 {
            return None;
        }

        // 0x00_00_01 or 0x00_00_00_01
        if self.buffer.len() < 3 || self.buffer[0] != 0 || self.buffer[1] != 0 {
            panic!("invalid NAL header")
        }
        match self.buffer[2] {
            1 => self.buffer = &self.buffer[3..],
            0 => {
                if self.buffer.len() < 4 || self.buffer[3] != 1 {
                    panic!("invalid NAL header")
                }
                self.buffer = &self.buffer[4..]
            }
            _ => panic!("invalid NAL header"),
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
                    return Some(nal);
                }
                _ => zero_count = 0,
            }
            index += 1
        }

        Some(std::mem::replace(&mut self.buffer, &[]))
    }
}
