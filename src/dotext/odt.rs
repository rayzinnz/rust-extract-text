use std::io;
use std::io::prelude::*;
use std::io::Cursor;
use std::path::{Path};

use super::doc::{self, OpenOfficeDoc};

pub struct Odt {
    data: Cursor<String>,
}

impl OpenOfficeDoc<Odt> for Odt {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Odt> {
        let text = doc::open_doc_read_data(path.as_ref(), "content.xml", &["text:p", "text:span"])?;

        Ok(Odt {
            data: Cursor::new(text),
        })
    }
}

impl Read for Odt {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data.read(buf)
    }
}
