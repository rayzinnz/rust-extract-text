//use log::*;
use zip::ZipArchive;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::path::Path;

pub trait MsDoc<T>: Read {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<T>;
}

pub trait OpenOfficeDoc<T>: Read {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<T>;
}

pub(crate) fn open_doc_read_data<P: AsRef<Path>>(
    path: P,
    content_name: &str,
    tags: &[&str],
) -> io::Result<String> {
    let file = File::open(path.as_ref())?;
    let mut archive = ZipArchive::new(file)?;

    let mut xml_data = String::new();

    for i in 0..archive.len() {
        let mut c_file = archive.by_index(i).unwrap();
        if c_file.name() == content_name {
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
                Ok(Event::Start(ref e)) => {
                    for tag in tags {
                        if e.name().as_ref() == tag.as_bytes() {
                            to_read = true;
                            if e.name().as_ref() == b"text:p" {
                                txt.push("\n\n".to_string());
                            }
                            break;
                        }
                    }
                }
                Ok(Event::Text(e)) => {
                    if to_read {
                        txt.push(e.decode().unwrap().into_owned());
                        to_read = false;
                    }
                }
                Ok(Event::Eof) => break,
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

    Ok(txt.join(""))
}
