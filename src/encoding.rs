use base64::Engine;

#[derive(Debug, Clone, Copy)]
pub enum Encoding {
    SevenBit,
    EightBit,
    Base64,
    QuotedPrintable,
}

impl Encoding {
    pub fn to_str(self) -> &'static str {
        match self {
            Encoding::SevenBit => "7bit",
            Encoding::EightBit => "8bit",
            Encoding::Base64 => "base64",
            Encoding::QuotedPrintable => "quoted-printable",
        }
    }

    pub fn encode(self, input: &mut Vec<u8>) {
        match self {
            Encoding::Base64 => {
                *input = base64::engine::general_purpose::STANDARD_NO_PAD
                    .encode(&input)
                    .into_bytes()
            }
            Encoding::QuotedPrintable => *input = quoted_printable::encode(&input),
            Encoding::SevenBit | Encoding::EightBit => (),
        }
    }
}
