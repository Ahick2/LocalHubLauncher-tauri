use encoding_rs::Encoding;

const MAX_PENDING: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Unknown,
    Bytes,
    Utf16Le,
    Utf16Be,
}

/// Newlines are recognized after determining the stream's code-unit width.
/// In particular, the NUL bytes in UTF-16 ASCII must not be treated as UTF-8.
pub struct LineDecoder {
    mode: Mode,
    pending: Vec<u8>,
    skip_lf: bool,
    skipping_long_line: bool,
    fallback: &'static Encoding,
}

impl Default for LineDecoder {
    fn default() -> Self {
        Self {
            mode: Mode::Unknown,
            pending: Vec::new(),
            skip_lf: false,
            skipping_long_line: false,
            fallback: ansi_encoding(),
        }
    }
}

impl LineDecoder {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        self.extract(false)
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.extract(true)
    }

    fn detect(&mut self, finished: bool) -> bool {
        if self.mode != Mode::Unknown {
            return true;
        }
        let bytes = &self.pending;
        if bytes.starts_with(&[0xff, 0xfe]) {
            self.mode = Mode::Utf16Le;
            self.pending.drain(..2);
        } else if bytes.starts_with(&[0xfe, 0xff]) {
            self.mode = Mode::Utf16Be;
            self.pending.drain(..2);
        } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            self.mode = Mode::Bytes;
            self.pending.drain(..3);
        } else {
            // A first byte can be a UTF-16 NUL or the first half of a newline.
            // Wait for a complete sample before committing to byte decoding.
            if !finished
                && bytes.len() < 4
                && (bytes.len() < 2 || !bytes.contains(&b'\n') || bytes.contains(&0))
            {
                return false;
            }
            let pairs: Vec<_> = bytes.chunks_exact(2).take(32).collect();
            let le = pairs.iter().filter(|pair| pair[1] == 0).count();
            let be = pairs.iter().filter(|pair| pair[0] == 0).count();
            self.mode = if !pairs.is_empty() && le * 2 >= pairs.len() && le > be {
                Mode::Utf16Le
            } else if !pairs.is_empty() && be * 2 >= pairs.len() && be > le {
                Mode::Utf16Be
            } else {
                Mode::Bytes
            };
        }
        true
    }

    fn extract(&mut self, finished: bool) -> Vec<String> {
        if self.pending.is_empty() || !self.detect(finished) {
            return Vec::new();
        }
        let width = if matches!(self.mode, Mode::Utf16Le | Mode::Utf16Be) {
            2
        } else {
            1
        };
        let mut output = Vec::new();
        let mut start = 0;
        let mut cursor = 0;
        while cursor + width <= self.pending.len() {
            let unit = match self.mode {
                Mode::Utf16Le => {
                    u16::from_le_bytes([self.pending[cursor], self.pending[cursor + 1]])
                }
                Mode::Utf16Be => {
                    u16::from_be_bytes([self.pending[cursor], self.pending[cursor + 1]])
                }
                _ => self.pending[cursor] as u16,
            };
            if self.skip_lf {
                self.skip_lf = false;
                if unit == 10 {
                    cursor += width;
                    start = cursor;
                    continue;
                }
            }
            if unit == 10 || unit == 13 {
                if !self.skipping_long_line {
                    output.push(self.decode(&self.pending[start..cursor]));
                }
                self.skipping_long_line = false;
                self.skip_lf = unit == 13;
                cursor += width;
                start = cursor;
            } else {
                cursor += width;
            }
        }
        if start > 0 {
            self.pending.drain(..start);
        }
        if self.pending.len() > MAX_PENDING {
            if !self.skipping_long_line {
                let mut text = self.decode(&self.pending[..MAX_PENDING]);
                text.push_str(" … [过长日志已截断]");
                output.push(text);
            }
            let consumed = self.pending.len() / width * width;
            self.pending.drain(..consumed);
            self.skipping_long_line = true;
        }
        if finished && !self.pending.is_empty() {
            if !self.skipping_long_line {
                output.push(self.decode(&self.pending));
            }
            self.pending.clear();
        }
        output
    }

    fn decode(&self, bytes: &[u8]) -> String {
        match self.mode {
            Mode::Utf16Le | Mode::Utf16Be => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| {
                        if self.mode == Mode::Utf16Le {
                            u16::from_le_bytes([pair[0], pair[1]])
                        } else {
                            u16::from_be_bytes([pair[0], pair[1]])
                        }
                    })
                    .collect();
                String::from_utf16_lossy(&units)
            }
            _ => std::str::from_utf8(bytes)
                .map(str::to_owned)
                .unwrap_or_else(|_| {
                    self.fallback
                        .decode_without_bom_handling(bytes)
                        .0
                        .into_owned()
                }),
        }
    }
}

fn ansi_encoding() -> &'static Encoding {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetACP;
        match unsafe { GetACP() } {
            936 => encoding_rs::GBK,
            932 => encoding_rs::SHIFT_JIS,
            949 => encoding_rs::EUC_KR,
            950 => encoding_rs::BIG5,
            65001 => encoding_rs::UTF_8,
            code => Encoding::for_label(format!("windows-{code}").as_bytes())
                .unwrap_or(encoding_rs::WINDOWS_1252),
        }
    }
    #[cfg(not(windows))]
    {
        encoding_rs::WINDOWS_1252
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_multibyte_characters_survive_arbitrary_chunk_boundaries() {
        let mut decoder = LineDecoder::default();
        let mut lines = Vec::new();
        for byte in "中文输出\r\nsecond\nlast".as_bytes() {
            lines.extend(decoder.feed(&[*byte]));
        }
        lines.extend(decoder.finish());
        assert_eq!(lines, ["中文输出", "second", "last"]);
    }

    #[test]
    fn utf16_ascii_is_decoded_before_any_ascii_fast_path() {
        let bytes: Vec<u8> = "Hello\r\nWorld\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut decoder = LineDecoder::default();
        let mut lines = Vec::new();
        for byte in bytes {
            lines.extend(decoder.feed(&[byte]));
        }
        lines.extend(decoder.finish());
        assert_eq!(lines, ["Hello", "World"]);
    }

    #[test]
    fn utf16_bom_and_chinese_are_handled_without_spurious_lines() {
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend("中文\r\n日志".encode_utf16().flat_map(u16::to_le_bytes));
        let mut decoder = LineDecoder::default();
        let mut lines = Vec::new();
        for chunk in bytes.chunks(3) {
            lines.extend(decoder.feed(chunk));
        }
        lines.extend(decoder.finish());
        assert_eq!(lines, ["中文", "日志"]);
    }

    #[test]
    fn utf16_without_bom_handles_a_split_leading_nul_or_newline() {
        for big_endian in [false, true] {
            let bytes: Vec<_> = "\nHello\r\n中文"
                .encode_utf16()
                .flat_map(|unit| {
                    if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect();
            let mut decoder = LineDecoder::default();
            let mut lines = Vec::new();
            for byte in bytes {
                lines.extend(decoder.feed(&[byte]));
            }
            lines.extend(decoder.finish());
            assert_eq!(lines, ["", "Hello", "中文"], "big_endian={big_endian}");
        }
    }

    #[test]
    fn a_stream_without_newlines_cannot_grow_memory_forever() {
        let mut decoder = LineDecoder::default();
        for _ in 0..100 {
            decoder.feed(&vec![b'x'; 8192]);
            assert!(decoder.pending.len() <= MAX_PENDING);
        }
        assert!(decoder.feed(b"\nnext\n").iter().any(|line| line == "next"));
    }
}
