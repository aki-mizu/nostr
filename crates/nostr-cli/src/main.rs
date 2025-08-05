// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2025 Rust Nostr Developers
// Distributed under the MIT software license

use std::fmt::Write;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use nostr_connect::prelude::*;
use nostr_relay_builder::prelude::*;
use nostr_sdk::prelude::*;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::{Config, Editor};
use tokio::time::Instant;

mod cli;
mod util;

use self::cli::{io, parser, Cli, Command, ShellCommand, ShellCommandDatabase};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
    }
}

async fn run() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        Command::Shell { relays } => {
            // Get data dir
            let data_dir: PathBuf = dirs::data_dir().expect("Can't find data directory");

            // Compose paths
            let nostr_cli_dir: PathBuf = data_dir.join("rust-nostr/cli");
            let db_path = nostr_cli_dir.join("data/lmdb");
            let history_path = nostr_cli_dir.join(".shell_history");

            // Create main dir if not exists
            fs::create_dir_all(nostr_cli_dir)?;

            // Open database
            let db: NostrLMDB = NostrLMDB::open(db_path)?;

            // Configure connection
            let connection: Connection = Connection::new()
                .target(ConnectionTarget::Onion)
                .embedded_tor();

            // Build client
            let opts: ClientOptions = ClientOptions::new().connection(connection);
            let client: Client = Client::builder().database(db).opts(opts).build();

            // Add relays
            for url in relays.iter() {
                client.add_relay(url).await?;
            }

            client.connect().await;

            let config = Config::builder().max_history_size(2000)?.build();
            let history = FileHistory::with_config(config);
            let rl: &mut Editor<(), FileHistory> = &mut Editor::with_history(config, history)?;

            // Load history
            let _ = rl.load_history(&history_path);

            loop {
                let readline = rl.readline("nostr> ");
                match readline {
                    Ok(line) => {
                        // Add to history
                        rl.add_history_entry(line.as_str())?;

                        // Split command line
                        let mut vec: Vec<String> = parser::split(&line)?;
                        vec.insert(0, String::new());

                        // Parse command
                        match ShellCommand::try_parse_from(vec) {
                            Ok(command) => {
                                if let Err(e) = handle_command(command, &client).await {
                                    eprintln!("Error: {e}");
                                }
                            }
                            Err(e) => {
                                eprintln!("{e}");
                            }
                        }
                        continue;
                    }
                    Err(ReadlineError::Interrupted) => {
                        // Ctrl-C
                        continue;
                    }
                    Err(ReadlineError::Eof) => break,
                    Err(e) => {
                        eprintln!("Error: {e}");
                        break;
                    }
                }
            }

            // Save history to file
            rl.save_history(&history_path)?;

            Ok(())
        }
        Command::Serve { port } => {
            let mut builder = RelayBuilder::default();

            if let Some(port) = port {
                builder = builder.port(port);
            }

            let relay = LocalRelay::run(builder).await?;

            println!("Relay running at {}", relay.url());

            loop {
                tokio::time::sleep(Duration::from_secs(60)).await
            }
        }
        Command::Bunker => {
            // Ask keys
            let keys = NostrConnectKeys {
                signer: io::get_keys("Signer Keys")?,
                user: io::get_keys("User Keys")?,
            };

            // Ask URI
            let uri: Option<String> = io::get_optional_input("Nostr Connect URI")?;

            // Compose signer
            let signer: NostrConnectRemoteSigner = match uri {
                Some(uri) => {
                    let uri: NostrConnectURI = NostrConnectURI::parse(&uri)?;
                    NostrConnectRemoteSigner::from_uri(uri, keys, None, None)?
                }
                None => NostrConnectRemoteSigner::new(keys, ["wss://relay.nsec.app"], None, None)?,
            };

            // Print bunker URI
            let uri: NostrConnectURI = signer.bunker_uri();
            println!("\nBunker URI: {uri}\n");

            // Serve signer
            signer.serve(CustomActions).await?;

            Ok(())
        }
    }
}

async fn handle_command(command: ShellCommand, client: &Client) -> Result<()> {
    match command {
        ShellCommand::Generate => {
            let keys: Keys = Keys::generate();
            println!("Secret key: {}", keys.secret_key().to_bech32()?);
            println!("Public key: {}", keys.public_key().to_bech32()?);
            Ok(())
        }
        ShellCommand::Sync {
            public_key,
            relays,
            direction,
        } => {
            let current_relays = client.relays().await;

            let list: Vec<RelayUrl> = if !relays.is_empty() {
                // Add relays
                for url in relays.iter() {
                    client.add_relay(url).await?;
                }

                println!("Connecting to relays...");

                // Connect and wait for connection
                client.try_connect(Duration::from_secs(60)).await;

                relays.clone()
            } else {
                current_relays.keys().cloned().collect()
            };

            println!("Syncing...");

            // Compose filter and opts
            let filter: Filter = Filter::default().author(public_key);
            let direction: SyncDirection = direction.into();
            let (tx, mut rx) = SyncProgress::channel();
            let opts: SyncOptions = SyncOptions::default().direction(direction).progress(tx);

            tokio::spawn(async move {
                let pb = ProgressBar::new(0);
                let style = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({percent_precise}%) - ETA: {eta}")
                    .unwrap()
                    .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
                    .progress_chars("#>-");
                pb.set_style(style);

                while rx.changed().await.is_ok() {
                    let SyncProgress { total, current } = *rx.borrow_and_update();
                    pb.set_length(total);
                    pb.set_position(current);
                }
            });

            // Reconcile
            let output: Output<Reconciliation> = client.sync_with(list, filter, &opts).await?;

            println!("Sync terminated:");
            println!("- Sent {} events", output.sent.len());
            println!("- Received {} events", output.received.len());

            // Remove relays
            for url in relays.into_iter() {
                if !current_relays.contains_key(&url) {
                    client.remove_relay(url).await?;
                }
            }

            Ok(())
        }
        ShellCommand::Query {
            id,
            author,
            kind,
            identifier,
            search,
            since,
            until,
            limit,
            database,
            print,
            json,
        } => {
            let db = client.database();

            let mut filter = Filter::new();

            if let Some(id) = id {
                filter = filter.id(id);
            }

            if let Some(author) = author {
                filter = filter.author(author);
            }

            if let Some(kind) = kind {
                filter = filter.kind(kind);
            }

            if let Some(identifier) = identifier {
                filter = filter.identifier(identifier);
            }

            if let Some(search) = search {
                filter = filter.search(search);
            }

            if let Some(since) = since {
                filter = filter.since(since);
            }

            if let Some(until) = until {
                filter = filter.until(until);
            }

            if let Some(limit) = limit {
                filter = filter.limit(limit);
            }

            if filter.is_empty() {
                eprintln!("Filters empty!");
            } else if database {
                // Query database
                let now = Instant::now();
                let events = db.query(filter).await?;

                let duration = now.elapsed();
                println!(
                    "{} results in {}",
                    events.len(),
                    if duration.as_secs() == 0 {
                        format!("{:.6} ms", duration.as_secs_f64() * 1000.0)
                    } else {
                        format!("{:.2} sec", duration.as_secs_f64())
                    }
                );
                if print {
                    // Print events
                    util::print_events(events, json);
                }
            } else {
                // Query relays
                println!("Querying relays...");
                
                let now = Instant::now();
                
                // Get events from relays
                let events = client
                    .fetch_events(filter, Duration::from_secs(10))
                    .await?;
                
                let duration = now.elapsed();
                println!(
                    "{} results in {}",
                    events.len(),
                    if duration.as_secs() == 0 {
                        format!("{:.6} ms", duration.as_secs_f64() * 1000.0)
                    } else {
                        format!("{:.2} sec", duration.as_secs_f64())
                    }
                );
                
                if print {
                    // Print events
                    util::print_events(events, json);
                }
            }

            Ok(())
        }
        ShellCommand::Database { command } => match command {
            ShellCommandDatabase::Populate { path } => {
                if path.exists() && path.is_file() {
                    // Open JSON file
                    let file = File::open(path)?;

                    let metadata = file.metadata()?;
                    let reader = BufReader::new(file);

                    println!("File size: {} bytes", metadata.len());

                    // Deserialize events
                    let iter = reader.lines().map_while(Result::ok).filter_map(|msg| {
                        if let Ok(RelayMessage::Event { event, .. }) = RelayMessage::from_json(msg)
                        {
                            Some(event)
                        } else {
                            None
                        }
                    });

                    // Bulk load
                    let mut counter: u32 = 0;
                    let db = client.database();
                    let now = Instant::now();

                    for event in iter {
                        if let Ok(status) = db.save_event(&event).await {
                            if status.is_success() {
                                counter += 1;
                            }
                        }
                    }

                    println!(
                        "Imported {counter} events in {:.6} secs",
                        now.elapsed().as_secs_f64()
                    );
                } else {
                    println!("File not found")
                }

                Ok(())
            }
            ShellCommandDatabase::Stats => {
                println!("TODO");
                Ok(())
            }
        },
        ShellCommand::Dvm {
            pubkey,
            param,
            content,
            max_results,
            timeout,
            print,
            json,
        } => {
            println!("Sending DVM request to {}...", pubkey);
            
            // Create and send DVM request event (kind 5300)
            let keys = io::get_keys("Your Keys")?;
            
            let mut event_builder = EventBuilder::new(Kind::Custom(5300), content)
                .tag(Tag::public_key(pubkey))
                .tag(Tag::parse(vec!["alt", "NIP90 Content Discovery Request"])?)
                .tag(Tag::parse(vec!["param", "max_results", &max_results.to_string()])?);
            
            // Add custom parameters
            for p in param {
                if let Some((key, value)) = p.split_once('=') {
                    event_builder = event_builder.tag(Tag::parse(vec!["param", key, value])?);
                }
            }
            
            // Add relay information
            let connected_relays: Vec<String> = client.relays().await.keys().map(|url| url.to_string()).collect();
            if !connected_relays.is_empty() {
                let mut relay_vec = vec!["relays"];
                relay_vec.extend(connected_relays.iter().map(|s| s.as_str()));
                event_builder = event_builder.tag(Tag::parse(relay_vec)?);
            }
            
            let event = event_builder.sign_with_keys(&keys)?;
            
            println!("Request event ID: {}", event.id);
            println!("Request content: '{}'", event.content);
            println!("Request tags:");
            for tag in event.tags.iter() {
                println!("  {:?}", tag);
            }
            
            // Send the request
            client.send_event(&event).await?;
            
            // Wait for DVM response with timeout
            println!("Waiting for DVM response...");
            
            let mut found_responses = 0;
            let start_time = Instant::now();
            let timeout_duration = Duration::from_secs(timeout);
            let max_attempts = ((timeout + 4) / 5) as u32; // Attempt every 5 seconds
            
            for attempt in 1..=max_attempts {
                if start_time.elapsed() >= timeout_duration {
                    println!("Timeout reached after {} seconds", timeout);
                    break;
                }
                
                println!("Checking for responses (attempt {}/{})...", attempt, max_attempts);
                
                // Wait a bit before each check (but respect timeout)
                let wait_time = if attempt == 1 { 3 } else { 5 };
                let remaining_time = timeout_duration.saturating_sub(start_time.elapsed());
                let actual_wait = Duration::from_secs(wait_time).min(remaining_time);
                
                if actual_wait > Duration::ZERO {
                    tokio::time::sleep(actual_wait).await;
                }
                
                // Check for responses that reference our request event ID  
                // Use the exact same filter construction as the working manual query
                let response_filter = Filter::new()
                    .kind(Kind::Custom(6300))
                    .author(pubkey)  // Changed from .pubkey() to .author()
                    .limit(10);
                
                println!("Querying relays directly...");
                
                // Try to get events from relays (similar to query --relay)
                let response_events = client
                    .fetch_events(response_filter.clone(), Duration::from_secs(10))
                    .await?;
                
                println!("Found {} total response events from this DVM", response_events.len());
                
                // If no events found, try querying the database too
                if response_events.is_empty() {
                    println!("No events from relays, trying database...");
                    let db_events = client.database().query(response_filter).await?;
                    println!("Found {} events in database", db_events.len());
                    
                    // Process database events
                    for response in db_events.into_iter() {
                        let is_our_response = response.tags.iter().any(|tag| {
                            if let Some(TagStandard::Event { event_id, .. }) = tag.as_standardized() {
                                return *event_id == event.id;
                            }
                            false
                        });
                        
                        if is_our_response {
                            found_responses += 1;
                            println!("🎉 Found matching response in database #{}!", found_responses);
                            println!("Response created at: {}", response.created_at.to_human_datetime());
                            println!("Response ID: {}", response.id);
                            
                            if print {
                                if json {
                                    println!("Response: {}", response.as_pretty_json());
                                } else {
                                    println!("Content length: {} characters", response.content.len());
                                    
                                    // Try to parse content as event IDs
                                    if let Ok(content_json) = serde_json::from_str::<Vec<Vec<String>>>(&response.content) {
                                        println!("📋 Event IDs from DVM ({} total):", content_json.len());
                                        for (i, tag) in content_json.iter().enumerate() {
                                            if tag.len() == 2 && tag[0] == "e" {
                                                println!("  {}. {}", i + 1, tag[1]);
                                            }
                                        }
                                    } else {
                                        println!("Content preview: {}", response.content.chars().take(200).collect::<String>());
                                    }
                                }
                            }
                            
                            // Found our response, we can exit
                            break;
                        }
                    }
                } else {
                    // Process relay events
                    for response in response_events.into_iter() {
                        let is_our_response = response.tags.iter().any(|tag| {
                            if let Some(TagStandard::Event { event_id, .. }) = tag.as_standardized() {
                                return *event_id == event.id;
                            }
                            false
                        });
                        
                        if is_our_response {
                            found_responses += 1;
                            println!("🎉 Found matching response #{}!", found_responses);
                            println!("Response created at: {}", response.created_at.to_human_datetime());
                            println!("Response ID: {}", response.id);
                            
                            if print {
                                if json {
                                    println!("Response: {}", response.as_pretty_json());
                                } else {
                                    println!("Content length: {} characters", response.content.len());
                                    
                                    // Try to parse content as event IDs
                                    if let Ok(content_json) = serde_json::from_str::<Vec<Vec<String>>>(&response.content) {
                                        println!("📋 Event IDs from DVM ({} total):", content_json.len());
                                        for (i, tag) in content_json.iter().enumerate() {
                                            if tag.len() == 2 && tag[0] == "e" {
                                                println!("  {}. {}", i + 1, tag[1]);
                                            }
                                        }
                                    } else {
                                        println!("Content preview: {}", response.content.chars().take(200).collect::<String>());
                                    }
                                }
                            }
                            
                            // Found our response, we can exit
                            break;
                        }
                    }
                }
                
                if found_responses > 0 {
                    break; // Found what we were looking for
                }
                
                if attempt < max_attempts {
                    println!("No matching responses yet, waiting...");
                }
            }
            
            if found_responses > 0 {
                println!("✅ Successfully received {} response(s) from DVM!", found_responses);
            } else {
                println!("❌ No responses found after {} seconds.", timeout);
                println!("You can manually check for responses with:");
                println!("query --kind 6300 --author {} --limit 5 --relay --print --json", pubkey);
            }
            
            Ok(())
        },
        ShellCommand::RelayList { pubkey, database, json } => {
            if database {
                let db = client.database();
                let filter = Filter::new()
                    .author(pubkey)
                    .kind(Kind::RelayList)
                    .limit(1);
                
                let events = db.query(filter).await?;
                
                if events.is_empty() {
                    println!("No relay list found for user {} in database", pubkey);
                } else {
                    let event = events.into_iter().next().unwrap();
                    print_relay_list(&event, json);
                }
            } else {
                println!("Querying relays for user's relay list...");
                
                let filter = Filter::new()
                    .author(pubkey)
                    .kind(Kind::RelayList)
                    .limit(1);
                
                let events = client
                    .fetch_events(filter, Duration::from_secs(10))
                    .await?;
                
                if events.is_empty() {
                    println!("No relay list found for user {}", pubkey);
                } else {
                    let event = events.into_iter().next().unwrap();
                    print_relay_list(&event, json);
                }
            }
            
            Ok(())
        },
        ShellCommand::Exit => std::process::exit(0x01),
    }
}

fn print_relay_list(event: &Event, json: bool) {
    use nostr_sdk::nips::nip65::extract_relay_list;
    
    if json {
        println!("{}", event.as_pretty_json());
        return;
    }
    
    println!("📡 Relay List for {}", event.pubkey);
    println!("Created: {}", event.created_at.to_human_datetime());
    println!("Event ID: {}", event.id);
    
    let mut read_relays = Vec::new();
    let mut write_relays = Vec::new();
    let mut readwrite_relays = Vec::new();
    
    for (relay_url, metadata) in extract_relay_list(event) {
        match metadata {
            Some(RelayMetadata::Read) => read_relays.push(relay_url),
            Some(RelayMetadata::Write) => write_relays.push(relay_url),
            None => readwrite_relays.push(relay_url),
        }
    }
    
    let total_relays = readwrite_relays.len() + read_relays.len() + write_relays.len();
    
    if !readwrite_relays.is_empty() {
        println!("\n🔄 Read/Write Relays ({}):", readwrite_relays.len());
        for relay in &readwrite_relays {
            println!("  • {}", relay);
        }
    }
    
    if !read_relays.is_empty() {
        println!("\n📖 Read Relays ({}):", read_relays.len());
        for relay in &read_relays {
            println!("  • {}", relay);
        }
    }
    
    if !write_relays.is_empty() {
        println!("\n✍️  Write Relays ({}):", write_relays.len());
        for relay in &write_relays {
            println!("  • {}", relay);
        }
    }
    
    println!("\nTotal relays: {}", total_relays);
}

struct CustomActions;

impl NostrConnectSignerActions for CustomActions {
    fn approve(&self, public_key: &PublicKey, req: &NostrConnectRequest) -> bool {
        println!("Public key: {public_key}");
        println!("{req:#?}\n");
        io::ask("Approve request?").unwrap_or_default()
    }
}
