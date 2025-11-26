use zip::ZipArchive;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::Cursor;
use std::path::{Path};

use super::doc::{MsDoc};

pub struct Docx {
    data: Cursor<String>,
}

impl MsDoc<Docx> for Docx {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Docx> {
        let file = File::open(path.as_ref())?;
        let mut archive = ZipArchive::new(file)?;

        let mut xml_data = String::new();

        for i in 0..archive.len() {
            let mut c_file = archive.by_index(i).unwrap();
            if c_file.name() == "word/document.xml" {
                c_file.read_to_string(&mut xml_data)?;
                break;
            }
        }

        let mut xml_reader = Reader::from_str(xml_data.as_ref());

        let mut buf = Vec::new();
        let mut txt = Vec::new();

        if xml_data.len() > 0 {
            let mut to_read = false;
            loop {
                match xml_reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref e)) => match e.name().as_ref() {
                        b"w:p" => {
                            to_read = true;
                            txt.push("\n\n".to_string());
                        }
                        b"w:t" => to_read = true,
                        _ => (),
                    },
                    Ok(Event::Text(e)) => {
                        if to_read {
                            txt.push(e.decode().unwrap().into_owned());
                            to_read = false;
                        }
                    }
                    Ok(Event::Eof) => break, // exits the loop when reaching end of file
                    Err(e) => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!(
                                "Error at position {}: {:?}",
                                xml_reader.buffer_position(),
                                e
                            ),
                        ))
                    }
                    _ => (),
                }
            }
        }

        Ok(Docx {
            data: Cursor::new(txt.join("")),
        })
    }
}

impl Read for Docx {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data.read(buf)
    }
}
