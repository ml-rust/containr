//! `AttachedFile` elements. The bytes are written apart from their headers, so
//! a large file is never copied a second time.

use crate::error::{Container, Error, Result};
use crate::mkv::attachments::MAX_ATTACHMENT_FIELD;
use crate::mkv::ebml_write::{ElementBuf, push_id, push_size};
use crate::mkv::ids;
use crate::mkv::mux::bytes::{derive_uid, invalid};
use crate::mkv::mux::options::DocType;

/// One file to attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentIn<'a> {
    /// `FileName`.
    pub name: &'a str,
    /// `FileMediaType`, such as `font/ttf`.
    pub mime: &'a str,
    /// `FileDescription`.
    pub description: Option<&'a str>,
    /// `FileUID`, kept when a tag targets the attachment. `None` derives one.
    pub uid: Option<u64>,
    /// `FileData`.
    pub data: &'a [u8],
}

/// One file the muxer holds until the finish.
#[derive(Debug)]
pub(crate) struct AttachedFile {
    pub uid: u64,
    /// The `AttachedFile` header and every child before the `FileData` body.
    head: Vec<u8>,
    data: Vec<u8>,
    /// The elements after the `FileData` body.
    tail: Vec<u8>,
}

impl AttachedFile {
    /// Check `file` and prepare its elements. `taken` holds the UIDs in use.
    pub fn new(file: &AttachmentIn<'_>, doc_type: DocType, taken: &[u64]) -> Result<Self> {
        if doc_type == DocType::WebM {
            return Err(Error::Unsupported {
                container: Container::WebM,
                what: format!("attachment `{}`: WebM has no attachments", file.name),
                use_instead: Container::Matroska,
            });
        }
        let description = file.description.unwrap_or_default();
        for (field, value) in [
            ("name", file.name),
            ("mime", file.mime),
            ("description", description),
        ] {
            if value.len() as u64 > MAX_ATTACHMENT_FIELD {
                return Err(invalid(format!(
                    "`AttachmentIn::{field}` holds {} bytes, above the limit of \
                     {MAX_ATTACHMENT_FIELD}. Use a shorter text",
                    value.len()
                )));
            }
        }
        let uid = match file.uid {
            Some(0) => {
                return Err(invalid(format!(
                    "attachment `{}` has `FileUID` 0. Use a non-zero UID, or `None`",
                    file.name
                )));
            }
            Some(uid) if taken.contains(&uid) => {
                return Err(invalid(format!(
                    "attachment `{}` repeats `FileUID` {uid}. Give each attachment its own UID",
                    file.name
                )));
            }
            Some(uid) => uid,
            None => derive_uid(file.name.as_bytes(), taken.len() as u64, |uid| {
                taken.contains(&uid)
            }),
        };
        let mut fields = ElementBuf::new();
        if let Some(description) = file.description {
            fields.text(ids::FILE_DESCRIPTION, description)?;
        }
        fields.text(ids::FILE_NAME, file.name)?;
        fields.text(ids::FILE_MEDIA_TYPE, file.mime)?;
        let mut data_head = Vec::with_capacity(11);
        push_id(&mut data_head, ids::FILE_DATA);
        push_size(&mut data_head, ids::FILE_DATA, file.data.len() as u64)?;
        let mut tail = ElementBuf::new();
        tail.uint(ids::FILE_UID, uid)?;
        let total = fields.as_bytes().len() as u64
            + data_head.len() as u64
            + file.data.len() as u64
            + tail.as_bytes().len() as u64;
        let mut head = Vec::with_capacity(fields.as_bytes().len() + data_head.len() + 12);
        push_id(&mut head, ids::ATTACHED_FILE);
        push_size(&mut head, ids::ATTACHED_FILE, total)?;
        head.extend_from_slice(fields.as_bytes());
        head.extend_from_slice(&data_head);
        Ok(Self {
            uid,
            head,
            data: file.data.to_vec(),
            tail: tail.into_bytes(),
        })
    }

    /// The whole element, in the order it goes out.
    pub fn parts(&self) -> [&[u8]; 3] {
        [
            self.head.as_slice(),
            self.data.as_slice(),
            self.tail.as_slice(),
        ]
    }

    /// The size of the whole element.
    pub fn len(&self) -> u64 {
        self.parts().iter().map(|part| part.len() as u64).sum()
    }
}

/// The `Attachments` header for `files`.
pub(crate) fn attachments_header(files: &[AttachedFile]) -> Result<Vec<u8>> {
    let total: u64 = files.iter().map(AttachedFile::len).sum();
    let mut out = Vec::with_capacity(12);
    push_id(&mut out, ids::ATTACHMENTS);
    push_size(&mut out, ids::ATTACHMENTS, total)?;
    Ok(out)
}
