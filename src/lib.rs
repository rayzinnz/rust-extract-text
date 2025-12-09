//! extract_text
//!
//! A small command-line utility for traversing a directory tree and preparing files
//! for text extraction. This module provides helpers for detecting file encodings,
//! handling archive-specific logic, and a top-level entry point which configures
//! logging and walks a directory recursively.


use calamine::{open_workbook_auto, DataType, Reader};
use cfb::CompoundFile;
use crc_fast::{checksum_file, CrcAlgorithm::Crc64Nvme};
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use encoding_rs_io::DecodeReaderBytesBuilder;
use log::*;
use mail_parser::{MessageParser, MimeHeaders};
use serde::{Serialize, Deserialize};
use sevenz_rust::decompress_file_with_password;
use std::{
	collections::HashSet,
	error::Error,
	fs::{self, File},
	io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
	path::{Path, PathBuf},
	process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use uuid::Uuid;
use walkdir::WalkDir;
use zip::ZipArchive;

mod ancillary;
use ancillary::tempfiles_location;

mod dotext;
use dotext::doc::{MsDoc, OpenOfficeDoc};
use dotext::docx::Docx;
use dotext::odt::Odt;

const DELETE_TEMP_FILES:bool = true;

struct MagicBytes {
	extension: &'static str,
	bytes: &'static [u8],
}

// https://en.wikipedia.org/wiki/List_of_file_signatures
const MAGIC_BYTES: [MagicBytes; 7] = [
	MagicBytes { extension: "7z", bytes: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] },
	MagicBytes { extension: "pdf", bytes: &[0x25, 0x50, 0x44, 0x46, 0x2D] },
	MagicBytes { extension: "zip", bytes: &[0x50, 0x4B, 0x03, 0x04] },
	MagicBytes { extension: "txt", bytes: &[0xEF, 0xBB, 0xBF] },
	MagicBytes { extension: "gzip", bytes: &[0x1F, 0x8B] },
	MagicBytes { extension: "txt", bytes: &[0xFE, 0xFF] },
	MagicBytes { extension: "txt", bytes: &[0xFF, 0xFE] },
];
// const IMAGE_MAGIC_BYTES: [MagicBytes; 1] = [
// 	MagicBytes { extension: "jpg", bytes: &[0xFF, 0xD8, 0xFF] },
// ];

const FILENAME_ILLEGAL_CHARS: [char; 9] = ['/' , '?' , '<' , '>' , '\\' , ':' , '*' , '|' , '"'];

// Constants for file extensions and size.
// For string literals, we use &str (string slices).
// const TEXT_ARCHIVE_EXTENSIONS: &[&str] = &[
// 	"msg",
// 	"eml",
// ];

pub const MAX_FILE_SIZE: u64 = 1_000_000_000; // 1GB in bytes

fn get_effective_file_extension(filepath: &Path) -> String {
	//handled extensions
	let file_extension = filepath.extension().unwrap_or_default().to_string_lossy().to_lowercase();

	if [
		"csv",
		"doc","docm","docx",
		"eml",
		"jpeg","jpg",
		"msg",
		"ods","odt",
		"pdf","png",
		"txt",
		"xlam","xls","xlsb","xlsm","xlsx","xlsx",
		].contains(&file_extension.as_str()) {
		return file_extension;
	}
	
	//magic bytes
	match filepath.metadata() {
		Ok(metadata) => {
			if metadata.len() < 16 {
				return file_extension;
			}
			match File::open(filepath) {
				Ok(mut file) => {
					let mut header = [0u8; 6];
					file.read_exact(&mut header).unwrap();
					for magic_bytes in MAGIC_BYTES {
						if *magic_bytes.bytes == header[0..magic_bytes.bytes.len()] {
							return String::from(magic_bytes.extension);
						}
					}
				}
				Err(e) => {
					error!("Error reading header bytes from file {:?}. {:?}", filepath, e);
					return file_extension;
				}
			}
		}
		Err(e) => {
			panic!("Error getting file metadata {:?}. {:?}", filepath, e);
		}
	}

	return file_extension;
}

fn read_file_with_encoding(filepath: &Path, encoding: &'static Encoding) -> Result<String, Box<dyn Error>> {
    let file = File::open(filepath)?;
	let mut decoder = DecodeReaderBytesBuilder::new()
        .encoding(Some(encoding)) // Specify the source encoding
        .build(file);
    let mut contents = String::new();
    decoder.read_to_string(&mut contents)?;

    Ok(contents)
}

/// Detects the encoding of a file based on its header bytes and content.
/// Specific use for use-case where two main types seen are CP1252 and UTF8. Other encoding detectors get confused sometimes, this one does not.
/// 
/// # Arguments
/// 
/// * `filepath` - A path to the file to detect encoding for
/// * `assume_utf8` - If true, assumes UTF-8 encoding when no BOM is found and content detection fails
/// 
/// # Returns
/// 
/// * EncodingDetection Enum. Checks for BOM first and resolves if any.
/// * Then if no BOM then UTF-8 when `assume_utf8` is true
/// * If `assume_utf8` is false, uses CP1252 encoding if opening file as UTF-8 fails
/// 
fn detect_encoding(filepath: &Path, assume_utf8: bool) -> &'static Encoding {
	//check if filepath exists and is a file
	if !filepath.exists() {
		return UTF_8;
	}
	// read the first 3 bytes of the file
	match File::open(filepath) {
		Ok(mut file) => {
			if let Ok(filemetadata) = filepath.metadata() {
				if filemetadata.len() > 3 {
					let mut header = [0u8; 3];
					// are the bytes utf8-bom ?
					file.read_exact(&mut header).unwrap();
					if header == [0xEF, 0xBB, 0xBF] {
						return UTF_8; //UTF_8 with BOM, Encoding does not have a BOM option for UTF_8
					}
					// are the first two byes of header utf-16-be?
					if header[0] == 0xFE && header[1] == 0xFF {
						return UTF_16BE;
					}
					// are the first two byes of header utf-16-le?
					if header[0] == 0xFF && header[1] == 0xFE {
						return UTF_16LE;
					}
				}
			}
			if assume_utf8 {
				return UTF_8;
			}
			//try read file as utf8. If error default to cp1252
			let mut reader = BufReader::new(file);
			reader.seek(SeekFrom::Start(0)).expect("Failed to seek");
			for line_result in reader.lines() {
				match line_result {
					Ok(_line_str) => {
						//info!("{:?}", line_str);
					}
					Err(e) => {
						debug!("detect_encoding utf8 detection failed: {:?}", e);
						return WINDOWS_1252;
					}
				}
			}
		}
		Err(e) => {
			error!("detect_encoding error: {:?}", e);
			return UTF_8;
		}
	}
	return UTF_8; // default encoding is UTF-8
}

// fn hex_to_bytes(s: &str) -> Result<Vec<u8>, std::num::ParseIntError> {
// 	(0..s.len())
// 		.step_by(2)
// 		.map(|i| u8::from_str_radix(&s[i..i + 2], 16))
// 		.collect()
// }

fn msg_get_contents(cfbf: &mut CompoundFile<File>, path: PathBuf) -> (String, String, Vec<PathBuf>) {
	let mut subject = String::new();
	let mut body = String::new();
	let mut sub_paths: Vec<PathBuf> = Vec::new();

	//subject 0x0037 Subject, 0x001F UTF_16LE
	if let Ok(mut stream) = cfbf.open_stream(path.join("__substg1.0_0037001F")) {
		let mut data = Vec::new();
		if let Ok(_) = stream.read_to_end(&mut data) {
			let data = UTF_16LE.decode(&data);
			// println!("{:?}", data);
			subject = data.0.to_string();
		}
	} else {
		panic!("Subject stream not found in {:?}", path)
	}

	//body 0x1000 Body, 0x001F UTF_16LE
	if let Ok(mut stream) = cfbf.open_stream(path.join("__substg1.0_1000001F")) {
		let mut data = Vec::new();
		if let Ok(_) = stream.read_to_end(&mut data) {
			let data = UTF_16LE.decode(&data);
			// println!("{:?}", data);
			body = data.0.to_string();
		}
	} else {
		panic!("Body stream not found in {:?}", path)
	}

	//attachments
	if let Ok(entries) = cfbf.read_storage(path) {
		for entry in entries {
			if entry.is_storage() {
				if entry.name().starts_with("__attach_") {
					// println!("{:?}", entry.path());
					let sub_path = entry.path().to_path_buf();
					sub_paths.push(sub_path);
				}
			}
		}
	}

	return (subject, body, sub_paths)
}

/// Produces a list of files held within files (if any), recursive, and extracts individual files within archives to a temp folder.
/// 
/// # Arguments
/// 
/// * `filepath` - A path to the top-level file to search for subfiles within
/// 
/// # Returns
/// 
/// * A heirarchal list of filepaths of any extracted files, includes the top-level file
fn extract_archive(filepath: &Path, depth:u8, parent_files: Vec<String>, list_of_files_in_archive: &mut Vec<SubFileItem>) -> Result<(), Box<dyn Error>> {


	debug!("filepath: {:?}", filepath);

	let achive_uuid_subdir: &str = &Uuid::new_v4().simple().to_string();

	//switch filepath extension
	let effective_file_extension = get_effective_file_extension(filepath);
	debug!("effective_file_extension: {:?}", effective_file_extension);

	
	match effective_file_extension.as_str() {
		"7z" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: false,
			});

			let outpath = tempfiles_location().join(&achive_uuid_subdir);
			// ignore returns and errors, if bad archive just skip
			let _ = decompress_file_with_password(filepath, &outpath, "a4".into());
			debug!("Extracted 7z to: {:?}", outpath);

			// Walk through all files and directories recursively
			for entry in WalkDir::new(outpath)
				.into_iter()
				.filter_map(|e| e.ok()) // Skip errors
			{
				let path = entry.path();
				if path.is_file() {
					let mut new_parent_files = parent_files.clone();
					new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
					// new_parent_files passes ownership instead of reference, because we no longer need it after passing into this function
					extract_archive(path, depth+1, new_parent_files, list_of_files_in_archive)?;
				}
			}
		}
		"docx" | "docm" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: true,
			});

			let file = File::open(filepath)?;
			let mut archive = zip::ZipArchive::new(file)?;

			for i in 0..archive.len() {
				let mut file = archive.by_index(i)?;
				let zipoutpath = match file.enclosed_name() {
					Some(path) => path.to_owned(),
					None => continue,
				};

				// Check if the file is in the 'word/media/' folder and has a typical image extension
				if zipoutpath.starts_with("word/media/") && 
				zipoutpath.extension().map_or(false, |ext| 
					ext == "png" || ext == "jpeg" || ext == "jpg") {

					let outpath = tempfiles_location().join(&achive_uuid_subdir).join(zipoutpath.file_name().unwrap());
					fs::create_dir_all(outpath.parent().unwrap())?;
					
					let mut outfile = File::create(&outpath)?;
					match io::copy(&mut file, &mut outfile) {
						Ok(_) => {
							let mut new_parent_files = parent_files.clone();
							new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
							extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
						},
						Err(e) => {
							error!("Error writing word image to file {:?}: {}", outpath, e)
						},
					}
				}
			}
		}
		"eml" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: false,
			});
			
			let mut file = File::open(filepath)?;
			let mut raw_email_data = Vec::new();
			file.read_to_end(&mut raw_email_data)?;

			let mut bodytext:String = String::new();
			if let Some(message) = MessageParser::default().parse(&raw_email_data) {
				if let Some(subject) = message.subject() {
					bodytext.push_str(subject);
				}
				if let Some(body) = message.body_text(0) {
					bodytext.push_str(&body);
				}
				let outpath = tempfiles_location().join(&achive_uuid_subdir).join("body.txt");
				fs::create_dir_all(outpath.parent().unwrap())?;
				match fs::write(&outpath, bodytext) {
					Ok(_) => {
						let mut new_parent_files = parent_files.clone();
						new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
						extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
					},
					Err(e) => {
						error!("Error writing to file {:?}: {}", outpath, e)
					},
				}
				
				for attachment in message.attachments() {
					let temp_filename = &Uuid::new_v4().simple().to_string();
					let attachment_name = attachment.attachment_name().unwrap_or(temp_filename);
					//println!("Attachment found: {}", attachment_name);
					let outpath = tempfiles_location().join(&achive_uuid_subdir).join(attachment_name);
					match fs::write(&outpath, attachment.contents()) {
						Ok(_) => {
							let mut new_parent_files = parent_files.clone();
							new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
							extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
						},
						Err(e) => {
							error!("Error writing to file {:?}: {}", outpath, e)
						},
					}

				}
			}
		}
		"msg" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: false,
			});

			let mut cfbf = cfb::open(filepath)?;

			let (subject, body, sub_paths) = msg_get_contents(&mut cfbf, PathBuf::from("/"));
			// debug!("{:?}", subject);
			// debug!("{:?}", body);
			// debug!("{:?}", sub_paths);

			let outpath = tempfiles_location().join(&achive_uuid_subdir).join("body.txt");
			fs::create_dir_all(outpath.parent().unwrap())?;
			let outtext = subject + "\n\n" + &body;
			match fs::write(&outpath, outtext) {
				Ok(_) => {
					let mut new_parent_files = parent_files.clone();
					new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
					extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
				},
				Err(e) => {
					error!("Error writing to file {:?}: {}", outpath, e)
				},
			}

			//stores the file subpath to write the output to and a list of cfbf subpaths
			let mut msg_attachments_to_traverse: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
			if !sub_paths.is_empty() {
				msg_attachments_to_traverse.push((PathBuf::new(), sub_paths.clone()));
			}

			while !msg_attachments_to_traverse.is_empty() {
				if let Some((filesubpath, sub_paths)) = msg_attachments_to_traverse.pop() {
					let achive_uuid_msg_subdir: &str = &Uuid::new_v4().simple().to_string();
					debug!("sub_paths: {:?}", sub_paths);
					for sub_path in sub_paths {
						debug!("depth: {}, path: {:?}", sub_path.components().count()-1, sub_path);
						// attachment binary, 0x3701 AttachDataObject, 0x0102 PT_BINARY
						if cfbf.exists(sub_path.join("__substg1.0_37010102")) {
							// println!("Binary attachment");
							//attachment filename, 0x3707 AttachLongFilename, 0x001F UTF_16LE
							let filename: String;
							if let Ok(mut stream) = cfbf.open_stream(sub_path.join("__substg1.0_3707001F")) {
								let mut data = Vec::new();
								stream.read_to_end(&mut data)?;
								let data = UTF_16LE.decode(&data);
								filename = data.0.to_string();
							} else {
								panic!("Body stream not found in {:?}", filepath)
							}
							//download binary attachment
							let mut stream = cfbf.open_stream(sub_path.join("__substg1.0_37010102"))?;
							let mut data = Vec::new();
							stream.read_to_end(&mut data)?;
							let outpath = tempfiles_location().join(&achive_uuid_subdir).join(achive_uuid_msg_subdir).join(filename);
							fs::create_dir_all(outpath.parent().unwrap())?;
							match fs::write(&outpath, data) {
								Ok(_) => {
									let mut new_parent_files = parent_files.clone();
									new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
									let parent_files_subpaths: Vec<String> = filesubpath.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
									new_parent_files.extend(parent_files_subpaths);
									extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
								},
								Err(e) => {
									error!("Error writing to file {:?}: {}", outpath, e)
								},
							}

						}
						//attachment msg path, 0x3701 AttachDataObject, 0x0102 PT_BINARY, 0x000D PT_OBJECT
						else if cfbf.exists(sub_path.join("__substg1.0_3701000D")) {
							// println!("MSG attachment");
							//attachment displayname, 0x3001 DisplayName, 0x001F UTF_16LE
							let mut displayname: String;
							if let Ok(mut stream) = cfbf.open_stream(sub_path.join("__substg1.0_3001001F")) {
								let mut data = Vec::new();
								stream.read_to_end(&mut data)?;
								let data = UTF_16LE.decode(&data);
								displayname = data.0.to_string();
							} else {
								panic!("Body stream not found in {:?}", filepath)
							}
							displayname.retain(|c| !FILENAME_ILLEGAL_CHARS.contains(&c));
							//empty file placeholder as embedded msg
							let msg_placeholder_filename = displayname.clone() + ".msg";
							let outpath = tempfiles_location().join(&achive_uuid_subdir).join(achive_uuid_msg_subdir).join(&msg_placeholder_filename);
							fs::create_dir_all(outpath.parent().unwrap())?;
							match fs::write(&outpath, "") {
								Ok(_) => {
									let mut new_parent_files = parent_files.clone();
									new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
									let parent_files_subpaths: Vec<String> = filesubpath.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
									new_parent_files.extend(parent_files_subpaths);
									list_of_files_in_archive.push(SubFileItem {
										filepath: outpath,
										depth,
										parent_files: new_parent_files.clone(),
										ok_to_extract_text: false,
									});
								},
								Err(e) => {
									error!("Error writing to file {:?}: {}", outpath, e)
								},
							}
							let filesubpath2 = filesubpath.clone().join(&msg_placeholder_filename);
							//recurse into path
							let (subject, body, sub_paths2) = msg_get_contents(&mut cfbf, sub_path.join("__substg1.0_3701000D"));
							let outpath = tempfiles_location().join(&achive_uuid_subdir).join(achive_uuid_msg_subdir).join("body.txt");
							fs::create_dir_all(outpath.parent().unwrap())?;
							let outtext = subject + "\n\n" + &body;
							match fs::write(&outpath, outtext) {
								Ok(_) => {
									let mut new_parent_files = parent_files.clone();
									new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
									let parent_files_subpaths: Vec<String> = filesubpath2.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
									new_parent_files.extend(parent_files_subpaths);
									extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
								},
								Err(e) => {
									error!("Error writing to file {:?}: {}", outpath, e)
								},
							}
							if !sub_paths2.is_empty() {
								msg_attachments_to_traverse.push((filesubpath2.clone(), sub_paths2.clone()));
							}
						}
						else {
							panic!("Unknown attachment type. Path: {:?}, file: {:?}", sub_path, filepath);
						}
					}
				}
			}
		}
		"odt" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: true,
			});

			let file = File::open(filepath)?;
			let mut archive = zip::ZipArchive::new(file)?;

			for i in 0..archive.len() {
				let mut file = archive.by_index(i)?;
				let zipoutpath = match file.enclosed_name() {
					Some(path) => path.to_owned(),
					None => continue,
				};

				// Check if the file is in the 'word/media/' folder and has a typical image extension
				if zipoutpath.starts_with("Pictures/") && 
				zipoutpath.extension().map_or(false, |ext| 
					ext == "png" || ext == "jpeg" || ext == "jpg") {

					let outpath = tempfiles_location().join(&achive_uuid_subdir).join(zipoutpath.file_name().unwrap());
					fs::create_dir_all(outpath.parent().unwrap())?;
					
					let mut outfile = File::create(&outpath)?;
					match io::copy(&mut file, &mut outfile) {
						Ok(_) => {
							let mut new_parent_files = parent_files.clone();
							new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
							extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
						},
						Err(e) => {
							error!("Error writing word image to file {:?}: {}", outpath, e)
						},
					}
				}
			}
		}
		"pdf" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: false,
			});

			fs::create_dir_all(tempfiles_location().join(&achive_uuid_subdir))?;

			// get page count
			let mut page_count: u32 = 0;
			let mut command = Command::new("pdfinfo");
			command.arg(format!("{}", filepath.to_str().expect("Path contains invalid UTF-8").to_string()));
			debug!("{:#?}", command);
			match command.output() {
				Ok(output) => {
					// println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
					// println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
					if !output.stderr.is_empty() {
						debug!("{:#?}", command);
						panic!("Error returned from {:?}: {}", command.get_program(), String::from_utf8_lossy(&output.stderr));
					}
					let output = String::from_utf8_lossy(&output.stdout);
					let output = output.lines();
					for line in output {
						if line.starts_with("Pages:") {
							let pc = line.split_whitespace();
							if let Some(pc) = pc.last() {
								let pc: u32 = pc.parse()?;
								page_count = pc;
							} else {
								println!("{:#?}", command);
								panic!("No page count found.");
							}
						}
					}
				}
				Err(e) => {
					println!("{:#?}", command);
					panic!("Failed to execute {:?}: {}", command.get_program(), e);
				}
			}
			if page_count == 0 {
				println!("{:#?}", command);
				panic!("Page count is 0");
			}
			trace!("PDF page count {}", page_count);
			for page_number in 1..=page_count {
				// debug!("page number: {}", page_number)

				//page text
				// pdftotext -f 1 -l 1 /home/ray/MEGA/Rays/Programming/python/file/test_text_extract/docs/sample2.pdf -
				// pdftotext -f 1 -l 1 -enc UTF-8 "C:\Users\hrag\Sync\Programming\python\file\test_text_extract\docs\fiche d'evaluation du stagiaire - Loïc Vital.pdf" C:\Users\hrag\AppData\Local\Temp\extract_text_from_file\pdftext.txt
				// https://www.xpdfreader.com/pdftotext-man.html
				let outpath = tempfiles_location().join(&achive_uuid_subdir).join(format!("page {}", page_number));
				let mut command = Command::new("pdftotext");
				command
					.arg("-f").arg(format!("{}", page_number))
					.arg("-l").arg(format!("{}", page_number))
					.arg(format!("{}", filepath.to_str().expect("Path contains invalid UTF-8").to_string()))
					.arg(format!("{}", outpath.to_str().expect("Path contains invalid UTF-8").to_string()));
				debug!("{:#?}", command);
				match command.output() {
					Ok(output) => {
						if !output.stderr.is_empty() {
							println!("{:#?}", command);
							panic!("Error returned from {:?}: {}", command.get_program(), String::from_utf8_lossy(&output.stderr));
						}
						let mut new_parent_files = parent_files.clone();
						new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
						extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
					}
					Err(e) => {
						println!("{:#?}", command);
						panic!("Failed to execute {:?}: {}", command.get_program(), e);
					}
				}

				//page images
				// pdfimages -list /home/ray/MEGA/Rays/Programming/python/file/test_text_extract/docs/sample2.pdf /tmp/extract_text_from_file/870eabfb3dc44ae185b84f6056f73397/image
				// pdfimages -list "C:\Users\hrag\Sync\Programming\python\file\test_text_extract\docs\fiche d'evaluation du stagiaire - Loïc Vital.pdf" C:\Users\hrag\AppData\Local\Temp\extract_text_from_file\image
				// https://www.xpdfreader.com/pdfimages-man.html
				let pdfimages_outpath = tempfiles_location().join(&achive_uuid_subdir).join(format!("page {} image", page_number));
				#[cfg(target_os = "windows")]
				{
					let mut command = Command::new("pdfimages");
					command
						.arg("-f").arg(format!("{}", page_number))
						.arg("-l").arg(format!("{}", page_number))
						.arg("-list")
						.arg(format!("{}", filepath.to_str().expect("Path contains invalid UTF-8").to_string()))
						.arg(format!("{}", pdfimages_outpath.to_str().expect("Path contains invalid UTF-8").to_string()));
					debug!("{:#?}", command);
					match command.output() {
						Ok(output) => {
							if !output.stderr.is_empty() {
								println!("{:#?}", command);
								panic!("Error returned from {:?}: {}", command.get_program(), String::from_utf8_lossy(&output.stderr));
							}
							//println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
							let output = String::from_utf8_lossy(&output.stdout);
							let output = output.lines();
							for line in output {
								if let Some((image_filename, _)) = line.split_once(": ") {
									// println!(">>> {}", image_filename);
									let outpath = PathBuf::from(image_filename);
									let mut new_parent_files = parent_files.clone();
									new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
									extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
								}
							}
						}
						Err(e) => {
							println!("{:#?}", command);
							panic!("Failed to execute {:?}: {}", command.get_program(), e);
						}
					}
				}
				#[cfg(target_os = "linux")]
				{
					//linux, first get list of images in page, then extract
					let mut command = Command::new("pdfimages");
					command
						.arg("-f").arg(format!("{}", page_number))
						.arg("-l").arg(format!("{}", page_number))
						.arg("-list")
						.arg(format!("{}", filepath.to_str().expect("Path contains invalid UTF-8").to_string()));
					debug!("{:#?}", command);
					match command.output() {
						Ok(output) => {
							if !output.stderr.is_empty() {
								println!("{:#?}", command);
								panic!("Error returned from {:?}: {}", command.get_program(), String::from_utf8_lossy(&output.stderr));
							}
							//println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
							let output = String::from_utf8_lossy(&output.stdout);
							let num_images = output.lines().count() - 2;
							// println!(">>> num_images {}", num_images);
							if num_images > 0 {
								//export
								let image_filename_prefix = pdfimages_outpath.to_str().expect("Path contains invalid UTF-8").to_string();
								let mut command = Command::new("pdfimages");
								command
									.arg("-f").arg(format!("{}", page_number))
									.arg("-l").arg(format!("{}", page_number))
									.arg(format!("{}", filepath.to_str().expect("Path contains invalid UTF-8").to_string()))
									.arg(format!("{}", image_filename_prefix));
								debug!("{:#?}", command);
								match command.output() {
									Ok(output) => {
										if !output.stderr.is_empty() {
											println!("{:#?}", command);
											panic!("Error returned from {:?}: {}", command.get_program(), String::from_utf8_lossy(&output.stderr));
										}
									}
									Err(e) => {
										println!("{:#?}", command);
										panic!("Failed to execute {:?}: {}", command.get_program(), e);
									}
								}
								for iimg in 0..num_images {
									let mut image_filename = image_filename_prefix.clone();
									image_filename.push_str(&format!("-{:03}.ppm", iimg));
									let outpath = PathBuf::from(image_filename);
									let mut new_parent_files = parent_files.clone();
									new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
									extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
								}
							}
						}
						Err(e) => {
							println!("{:#?}", command);
							panic!("Failed to execute {:?}: {}", command.get_program(), e);
						}
					}

				}
			}

		}
		"ods" | "xlam" | "xls" | "xlsb" | "xlsm" | "xlsx" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: false,
			});
			let mut workbook = open_workbook_auto(filepath)?;

			if let Ok(vbaop) = workbook.vba_project() {
				if let Some(vba) = vbaop {
					let vba_modules = vba.get_module_names();
					trace!("vba_modules: {:#?}", vba_modules);
					for module_name in vba_modules {
						let module = vba.get_module(module_name).unwrap();
						let mut module_name_filename_safe = module_name.to_string();
						module_name_filename_safe.retain(|c| !FILENAME_ILLEGAL_CHARS.contains(&c));
						let outpath = tempfiles_location().join(&achive_uuid_subdir).join(format!("VBA_{}", module_name_filename_safe));
						fs::create_dir_all(outpath.parent().unwrap())?;
						match fs::write(&outpath, module) {
							Ok(_) => {
								let mut new_parent_files = parent_files.clone();
								new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
								extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
							},
							Err(e) => {
								error!("Error writing to file {:?}: {}", outpath, e)
							},
						}
					}
				}
			}

			let sheets_metadata = workbook.sheets_metadata().to_owned();
			for sheet in sheets_metadata {
				let mut text: String = String::new();
				// trace!("sheet_metadata: {:?}", sheet);
				if sheet.typ == calamine::SheetType::WorkSheet {
					trace!("Reading sheet: {}", sheet.name);
					if let Ok(range) = workbook.worksheet_range(&sheet.name) {
						for row in range.rows() {
							let mut line: String = String::new();
							for (icell, cell) in row.iter().enumerate() {
								if icell>0 {
									line.push_str("\t");
								}
								line.push_str(cell.as_string().unwrap_or_default().as_str());
							}
							if !line.trim().is_empty() {
								line.push_str("\n");
								text.push_str(&line);
							}
						}
					}

					if !text.is_empty() {
						let mut sheet_name_filename_safe = sheet.name.clone();
						sheet_name_filename_safe.retain(|c| !FILENAME_ILLEGAL_CHARS.contains(&c));
						let outpath = tempfiles_location().join(&achive_uuid_subdir).join(format!("{}", sheet_name_filename_safe));
						fs::create_dir_all(outpath.parent().unwrap())?;
						match fs::write(&outpath, text) {
							Ok(_) => {
								let mut new_parent_files = parent_files.clone();
								new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
								extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
							},
							Err(e) => {
								error!("Error writing to file {:?}: {}", outpath, e)
							},
						}
					}
				} else {
					trace!("Skipping sheet {} of type {:?}", sheet.name, sheet.typ);
				}
			}

		}
		"zip" => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: false,
			});
			
			let file = File::open(filepath)?;
			let mut archive = ZipArchive::new(file)?;
			debug!("Total entries: {}", archive.len());
			for i in 0..archive.len() {
				let mut zipfile = archive.by_index(i)?;
				// debug!("  {}: {} ({} bytes)", i, zipfile.name(), zipfile.size());
				let outpath = tempfiles_location().join(&achive_uuid_subdir).join(zipfile.mangled_name());
				if zipfile.is_dir() {
					fs::create_dir_all(&outpath)?;
					// debug!("Created directory: {:?}", outpath);
				} else {
					// Handle files
					if let Some(parent) = outpath.parent() {
						fs::create_dir_all(parent)?;
					}

					// Extract the file
					let mut outfile = File::create(&outpath)?;
					io::copy(&mut zipfile, &mut outfile)?;
					debug!("Extracted: {:?}", outpath);
					let mut new_parent_files = parent_files.clone();
					new_parent_files.push(filepath.file_name().unwrap_or_default().to_string_lossy().to_string());
					// new_parent_files passes ownership instead of reference, because we no longer need it after passing into this function
					extract_archive(outpath.as_path(), depth+1, new_parent_files, list_of_files_in_archive)?;
					//filepath.file_name().unwrap_or_default().to_string_lossy().to_string()
				}
			}
		}
		_ => {
			list_of_files_in_archive.push(SubFileItem {
				filepath: filepath.to_path_buf(),
				depth,
				parent_files: parent_files.clone(),
				ok_to_extract_text: true,
			});
			
		}
	}


	Ok(())
}

fn ocr(filepath: &Path) -> Result<String, Box<dyn Error>> {
	// tesseract -l eng "C:\Users\hrag\AppData\Local\Temp\extract_text_from_file\43766efc4742438884b0f109fd6a6bac\image-0001.ppm" C:\Users\hrag\AppData\Local\Temp\extract_text_from_file\43766efc4742438884b0f109fd6a6bac\ocr
	// https://tesseract-ocr.github.io/tessdoc/Command-Line-Usage.html
	// https://github.com/tesseract-ocr/tessdata_fast
	// get traineddata for eng (english) and osd (orientation and script detection)
	let a_uuid: &str = &Uuid::new_v4().simple().to_string();
	let outpath = tempfiles_location().join(a_uuid);
	let mut outpath = format!("{}", outpath.to_str().expect("Path contains invalid UTF-8").to_string());
	let mut command = Command::new("tesseract");
	command
		.arg("-l").arg("eng")
		.arg(format!("{}", filepath.to_str().expect("Path contains invalid UTF-8").to_string()))
		.arg(&outpath);
	trace!("{:#?}", command);
	match command.output() {
		Ok(_output) => {
			//println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
		}
		Err(e) => {
			println!("{:#?}", command);
			panic!("Failed to execute {:?}: {}", command.get_program(), e);
		}
	}
	outpath.push_str(&".txt");
	let outpath = PathBuf::from(outpath);
	if outpath.exists() {
		let contents = read_text_from_file(&outpath)?;
		_ = std::fs::remove_file(&outpath);
		return Ok(contents);
	}

	return Ok(String::new());
}

fn convert_accented_manual(s: &str) -> String {
	s.chars()
		.map(|c| match c {
			'á' | 'à' | 'ã' | 'â' => 'a',
			'é' | 'è' | 'ê' => 'e',
			'í' | 'ì' | 'î' | 'ï' => 'i',
			'ó' | 'ò' | 'õ' | 'ô' => 'o',
			'ú' | 'ù' | 'ũ' | 'û' => 'u',
			'ñ' => 'n',
			// Add more mappings as needed
			_ => c, // Keep other characters as they are
		})
		.collect()
}

fn read_text_from_file(filepath: &Path) -> Result<String, Box<dyn Error>> {
	let file_encoding = detect_encoding(filepath, false);
	debug!("file_encoding: {:?}", file_encoding);
	let mut contents = read_file_with_encoding(filepath, file_encoding)?;
	// if file_encoding == WINDOWS_1252 {
		//if no 0 or 255 bytes the in the contents, assume this is a text file and convert accented characters to base letters
		if !(contents.as_bytes().contains(&0) || contents.as_bytes().contains(&255)) {
			contents = convert_accented_manual(&contents);
		}
		//clean all but english letters
		contents.retain(|c| c.is_ascii_graphic() || c.is_whitespace());
	// }
	// debug!("contents: {:?}", contents);
	return Ok(contents);
}

#[allow(dead_code)]
#[derive(Debug)]
struct SubFileItem {
	filepath: PathBuf,
	depth: u8,
	parent_files: Vec<String>,
	ok_to_extract_text: bool,
}

fn extract_text_from_subfile(file_list_item: &SubFileItem) -> Result<String, Box<dyn Error>> {
	debug!("subfile to extract text: {:?}", file_list_item.filepath);
	
	let file_extension = file_list_item.filepath.extension().unwrap_or_default().to_string_lossy().to_lowercase();
	if !file_list_item.ok_to_extract_text {
		return Ok(String::new())
	}

	match file_extension.as_str() {
		"docx" | "docm" => {
			//dotext
			match <Docx as MsDoc<Docx>>::open(file_list_item.filepath.as_path()) {
				Ok(mut doc) => {
					let mut text = String::new();
					let _ = doc.read_to_string(&mut text);
					return Ok(text);
				}
				Err(e) => {
					warn!("Error extracting text from docx {:?}\n{:?}", file_list_item.filepath, e);
					return Ok(String::new());
				}
			}
		}
		"odt" => {
			//dotext
			match <Odt as OpenOfficeDoc<Odt>>::open(file_list_item.filepath.as_path()) {
				Ok(mut doc) => {
					let mut text = String::new();
					let _ = doc.read_to_string(&mut text);
					return Ok(text);
				}
				Err(e) => {
					warn!("Error extracting text from docx {:?}\n{:?}", file_list_item.filepath, e);
					return Ok(String::new());
				}
			}
		}
		"jpeg"| "jpg" | "pgm" | "png" | "ppm" => {
			//tesseract
			match ocr(file_list_item.filepath.as_path()) {
				Ok(extracted_text) => {
					return Ok(extracted_text);
				}
				Err(e) => {
					warn!("Error extracting text from image {:?}\n{:?}", file_list_item.filepath, e);
					return Ok(String::new());
				}
			}
			// return Ok(String::new());
		}
		_ => {
			//text
			let contents = read_text_from_file(file_list_item.filepath.as_path())?;
			// debug!("contents: {:?}", contents);
			return Ok(contents);
		}
	}
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct FileListItem {
	pub filename: String,
	pub parent_files: Vec<String>,
	pub crc: i64,
	pub size: i64,
	pub text_contents: Option<String>
}

pub fn extract_text_from_file(filepath: &Path, pre_scanned_items: Vec<FileListItem>, keep_going: Arc<AtomicBool>) -> Result<Vec<FileListItem>, Box<dyn Error>> {
	let mut list_of_files_in_archive: Vec<SubFileItem> = Vec::new();
	let parent_files: Vec<String> = Vec::new();
	extract_archive(filepath, 0, parent_files, &mut list_of_files_in_archive)?;

	// debug!("list_of_files_in_archive: {:#?}", list_of_files_in_archive);

	let mut file_list_items: Vec<FileListItem> = Vec::new();

	//loop list_of_files_in_archive
	let mut temp_dirs_to_remove: HashSet<PathBuf> = HashSet::new();
	for sub_file_item in list_of_files_in_archive {
		match sub_file_item.filepath.metadata() {
			Ok(metadata) => {
				let file_name = sub_file_item.filepath.file_name().unwrap().to_string_lossy().to_string();
				let file_len:u64 = metadata.len();
				trace!("file_len {}", file_len);
				if file_len==0 {
					//add a SubFileItem with empty contents.
					let file_list_item: FileListItem = FileListItem{
						filename: file_name,
						parent_files: sub_file_item.parent_files,
						crc: 0,
						size: file_len as i64,
						text_contents: Some(String::new()),
					};
					file_list_items.push(file_list_item);
					continue;
				}
				debug!("{:?}", sub_file_item);
				debug!("\n  file: {:?}\n    depth:{}, {:?}\n      subfile: {:?}", filepath, sub_file_item.depth, sub_file_item.parent_files, sub_file_item.filepath.file_name().unwrap());

				let file_crc: i64 = checksum_file(Crc64Nvme, sub_file_item.filepath.to_str().unwrap(), None).unwrap() as i64;

				//if this is in a prescanned item, then check the filecrc
				let mut skip_file = false;
				for prescanned_item in &pre_scanned_items {
					if prescanned_item.filename == file_name
						&& prescanned_item.parent_files == sub_file_item.parent_files
						&& prescanned_item.crc == file_crc
					{
						info!("Sub file not changed, skipping...");
						skip_file = true;
						break;
					}
				}
				
				if skip_file {
					let file_list_item: FileListItem = FileListItem{
						filename: file_name,
						parent_files: sub_file_item.parent_files,
						crc: file_crc,
						size: file_len as i64,
						text_contents: None,
					};
					file_list_items.push(file_list_item);
				} else {
					let subfile_text = extract_text_from_subfile(&sub_file_item)?;
					// trace!("subfile_text {:?}", subfile_text);
					//cleanup of temp files and dirs
					if DELETE_TEMP_FILES {
						if sub_file_item.depth >= 1 {
							let temp_dir = sub_file_item.filepath.clone();
							let temp_dir = temp_dir.parent().unwrap().to_path_buf();
							temp_dirs_to_remove.insert(temp_dir);
							_ = std::fs::remove_file(&sub_file_item.filepath); //delete the file
						}
					}
					let file_list_item: FileListItem = FileListItem{
						filename: file_name,
						parent_files: sub_file_item.parent_files,
						crc: file_crc,
						size: file_len as i64,
						text_contents: Some(subfile_text),
					};
// println!("file_list_item: {:?}", file_list_item);
					file_list_items.push(file_list_item);
				}
			}
			Err(e) => {
				panic!("Error getting metadata for file: {:?} error: {:?}", sub_file_item.filepath, e);
			}
		}

		if !keep_going.load(Ordering::Relaxed) {
			break;
		}
	}
	//remove temp folders
	for temp_dir in temp_dirs_to_remove {
		_ = std::fs::remove_dir_all(&temp_dir); //delete the temp dir
	}

	Ok(file_list_items)
}

#[cfg(test)]
mod tests {
	use super::*;

    #[test]
    fn extract_text_from_file_empty_file() {
		let pre_scanned_items: Vec<FileListItem> = Vec::new();
		let keep_going = Arc::new(AtomicBool::new(true));
		let keep_going_flag = keep_going.clone();
		let result = extract_text_from_file(
			Path::new("./tests/resources/files_to_scan/empty_file"),
			pre_scanned_items,
			keep_going_flag
		).unwrap();
		//load expected from serde serialization
		let serial_path = Path::new("./tests/resources/expected/empty_file.json");
		let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
		let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		
		assert_eq!(result, expected);
    }

	#[test]
    fn extract_text_from_file_txt_utf8() {
		let pre_scanned_items: Vec<FileListItem> = Vec::new();
		let keep_going = Arc::new(AtomicBool::new(true));
		let keep_going_flag = keep_going.clone();
		let result = extract_text_from_file(
			Path::new("./tests/resources/files_to_scan/txt/text_utf8.txt"),
			pre_scanned_items,
			keep_going_flag
		).unwrap();
		// //load expected from serde serialization
		// let serial_path = Path::new("./tests/resources/expected/empty_file.json");
		// let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
		// let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		//check each byte of contents
		println!("*** {}", result.len());
		
		// assert_eq!(result, None);
    }

    #[cfg(target_os = "windows")]
	#[test]
    fn extract_text_from_file_docs_5407953830_pdf() {
		let pre_scanned_items: Vec<FileListItem> = Vec::new();
		let keep_going = Arc::new(AtomicBool::new(true));
		let keep_going_flag = keep_going.clone();
		let result = extract_text_from_file(
			Path::new("./tests/resources/files_to_scan/docs/5407953830.pdf"),
			pre_scanned_items,
			keep_going_flag
		).unwrap();
		//load expected from serde serialization
		let serial_path = Path::new("./tests/resources/expected/docs/5407953830.pdf.windows.json");
		let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
		let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		
		assert_eq!(result, expected);
    }

    #[cfg(target_os = "linux")]
	#[test]
    fn extract_text_from_file_docs_5407953830_pdf() {
		let pre_scanned_items: Vec<FileListItem> = Vec::new();
		let keep_going = Arc::new(AtomicBool::new(true));
		let keep_going_flag = keep_going.clone();
		let result = extract_text_from_file(
			Path::new("./tests/resources/files_to_scan/docs/5407953830.pdf"),
			pre_scanned_items,
			keep_going_flag
		).unwrap();
		//load expected from serde serialization
		let serial_path = Path::new("./tests/resources/expected/docs/5407953830.pdf.linux.json");
		let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
		let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		
		assert_eq!(result, expected);
    }

    #[cfg(target_os = "windows")]
	#[test]
    fn extract_text_from_file_emails_msg_in_msg() {
		let pre_scanned_items: Vec<FileListItem> = Vec::new();
		let keep_going = Arc::new(AtomicBool::new(true));
		let keep_going_flag = keep_going.clone();
		let result = extract_text_from_file(
			Path::new("./tests/resources/files_to_scan/emails/msg_in_msg.msg"),
			pre_scanned_items,
			keep_going_flag
		).unwrap();
		//load expected from serde serialization
		let serial_path = Path::new("./tests/resources/expected/emails/msg_in_msg.msg.windows.json");
		let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
		let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		
		assert_eq!(result, expected);
    }

    #[cfg(target_os = "linux")]
	#[test]
    fn extract_text_from_file_emails_msg_in_msg() {
		let pre_scanned_items: Vec<FileListItem> = Vec::new();
		let keep_going = Arc::new(AtomicBool::new(true));
		let keep_going_flag = keep_going.clone();
		let result = extract_text_from_file(
			Path::new("./tests/resources/files_to_scan/emails/msg_in_msg.msg"),
			pre_scanned_items,
			keep_going_flag
		).unwrap();
		//load expected from serde serialization
		let serial_path = Path::new("./tests/resources/expected/emails/msg_in_msg.msg.linux.json");
		let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
		let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		
		assert_eq!(result, expected);
    }

	//this one is large and slow
	// #[test]
    // fn extract_text_from_file_emails_msg_in_msg_in_msg() {
	// 	let pre_scanned_items: Vec<FileListItem> = Vec::new();
	// 	let keep_going = Arc::new(AtomicBool::new(true));
	// 	let keep_going_flag = keep_going.clone();
	// 	let result = extract_text_from_file(
	// 		Path::new("./tests/resources/files_to_scan/emails/msg_in_msg_in_msg.msg"),
	// 		pre_scanned_items,
	// 		keep_going_flag
	// 	).unwrap();
	// 	//load expected from serde serialization
	// 	let serial_path = Path::new("./tests/resources/expected/emails/msg_in_msg_in_msg.msg.json");
	// 	let obj_as_json = fs::read_to_string(serial_path).expect("Error reading serialized file.");
	// 	let expected: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		
	// 	assert_eq!(result, expected);
    // }

}
