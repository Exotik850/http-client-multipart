use std::borrow::Cow;

use http_types::Body;

/// Represents a single field in a multipart form.
#[derive(Debug)]
pub enum Part<'p> {
    /// A text field.
    Text {
        name: Cow<'p, str>,
        value: Cow<'p, str>,
    },
    /// A file field.
    File {
        name: Cow<'p, str>,
        filename: Cow<'p, str>,
        content_type: Cow<'p, str>,
        encoding: Option<ContentTransferEncoding>,
        data: Body,
    },
}


#[derive(Debug, Clone, Copy)]
pub enum ContentTransferEncoding {
    SevenBit,
    EightBit,
    Base64,
    QuotedPrintable,
}

impl ContentTransferEncoding {
    pub fn to_str(&self) -> &'static str {
        match self {
            ContentTransferEncoding::SevenBit => "7bit",
            ContentTransferEncoding::EightBit => "8bit",
            ContentTransferEncoding::Base64 => "base64",
            ContentTransferEncoding::QuotedPrintable => "quoted-printable",
        }
    }
}
