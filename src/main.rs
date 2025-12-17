use crc_fast::{checksum_file, CrcAlgorithm::Crc64Nvme};
use extract_text::*;
use helper_lib::{
	watch_for_quit,
	paths::add_extension
};
use log::*;
use serde_json;
use simplelog::*;
use std::{
	error::Error,
	fs,
	path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
	thread,
};

fn main()  -> Result<(), Box<dyn Error>> {
    let logger_config = ConfigBuilder::new()
		.set_time_offset_to_local().expect("Failed to get local time offset")
		.set_time_format_custom(format_description!("[hour]:[minute]:[second].[subsecond digits:3]"))
        .build();
	CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Trace, logger_config, TerminalMode::Mixed, ColorChoice::Auto),
			// TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
            // WriteLogger::new(LevelFilter::Info, Config::default(), File::create("my_rust_binary.log").unwrap()),
        ]
    ).unwrap();

	// error!("Bright red error");
    // info!("This only appears in the log file");
    // debug!("This level is currently not enabled for any logger");

    let keep_going = Arc::new(AtomicBool::new(true));
    let keep_going_flag = keep_going.clone();
    let _watch_for_quit_handle = thread::spawn(move || {watch_for_quit(keep_going_flag);});

	let starting_path: PathBuf = PathBuf::from("./tests/resources/files_to_scan");

    info!("Starting to traverse directory: {:?}", starting_path);
    
    // Walk through all files and directories recursively
    // for entry in WalkDir::new(starting_path)
    //     .into_iter()
    //     .filter_map(|e| e.ok()) // Skip errors
    // {
    //     let path = entry.path();
        
    //     // Process only files (not directories)
    //     if path.is_file() && path.metadata()?.len() < MAX_FILE_SIZE {
	// 		//println!("path: {:?}", path);
	// 		extract_text_from_file(path)?;
    //     }
    // }

	// subpath starts from under here: ./tests/resources/files_to_scan
	// let subpath = Path::new("empty_file");
	// let subpath = Path::new("archives/EICAR_test_virus.TXT.zip");
	// let subpath = Path::new("archives/ArtemisTestVirusWithSignedExes.7z");
	// let subpath = Path::new("archives/SSMS18.7z");
	// let subpath = Path::new("binary/fpext.msg");
	// let subpath = Path::new("txt/text_utf8.txt");
	// let subpath = Path::new("txt/text_utf16le.txt");
	// let subpath = Path::new("docs/pass_protected_with_readable_text.xls");
	// let subpath = Path::new("docs/pass_protected.ods");
	// let subpath = Path::new("docs/pass_protected.xlsx");
	// let subpath = Path::new("docs/pass_protected.xlsb");
	// let subpath = Path::new("docs/231007 - P-2 use.xls");
	// let subpath = Path::new("docs/IC3_231019_gradient.xls");
	// let subpath = Path::new("docs/CPROD - 13NZAK0060930 - 20130927.xlsx");
	// let subpath = Path::new("docs/5407953830.pdf");
	// let subpath = Path::new("docs/ImageFusion_Module_User_Guide.pdf");
	// let subpath = Path::new("docs/ILEADER-V4 3-User Manual-Administration Module-1.0.0.pdf");
	// let subpath = Path::new("docs/Geoforce - pointage - flux vers Chronos v2.pdf");
	// let subpath = Path::new("docs/Developmental-History-Form.pdf");
	// let subpath = Path::new("docs/Testing.docx");
	// let subpath = Path::new("emails/msg_in_msg_in_msg.msg");
	let subpath = Path::new("emails/msg_in_msg.msg");
	// let subpath = Path::new("emails/test_email_1.msg");
	// let subpath = Path::new("emails/COD eLIMS.msg");

	// let path = Path::new(r"C:\Users\hrag\Sync\work\Auditing\iLeader\iLeader Docs.7z");
	let path = Path::new("./tests/resources/files_to_scan").join(subpath);
	let file_crc = checksum_file(Crc64Nvme, path.to_str().unwrap(), None).unwrap() as i64;
	debug!("file_crc: {}", file_crc);
	let pre_scanned_items: Vec<FileListItem> = Vec::new();
	let keep_going_flag = keep_going.clone();
	let contents = extract_text_from_file(&path, pre_scanned_items, keep_going_flag)?;

	debug!("{:#?}", contents);

	// let text_contents = contents.first().unwrap().text_contents.as_ref().unwrap();
	// println!("{}", text_contents);
	// println!("{}", text_contents.len());
	// for b in text_contents.as_bytes() {
	// 	print!("{}-", b);
	// }
	// println!();

	let store_serialized_contents_to_testing_file = false;
	if store_serialized_contents_to_testing_file {
		//store serialized contents to file
		let mut serial_path = Path::new("./tests/resources/expected").join(subpath);
		serial_path = add_extension(&serial_path, "json");
		fs::create_dir_all(&serial_path.parent().unwrap()).expect("Error creating path for serialized file");
		let serialized = serde_json::to_string_pretty(&contents).expect("Error when serializing contents object.");
		// debug!("{}", serialized);
		fs::write(&serial_path, serialized).expect("Could not write serialize file.");
		//load serialized object
		let obj_as_json = fs::read_to_string(&serial_path).expect("Error reading serialized file.");
		let _contents: Vec<FileListItem> = serde_json::from_str(&obj_as_json).expect("Error loading serialized json.");
		// debug!("{:#?}", contents);
	}

    info!("Finished traversing directory");
    
	keep_going.store(false, Ordering::Relaxed);
	#[cfg(target_os = "linux")]
	if let Err(e) = _watch_for_quit_handle.join() {
		error!("watch_for_quit thread join error: {:?}", e);
	}

	// keep_going.store(false, Ordering::Relaxed);

    Ok(())
}
