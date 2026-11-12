//! Startup shared by the gameplay library and its executable.
use crate::{crash, game, logging};

/// Start the game. The executable supplies the allocator and retains the mod API marker.
pub fn main() {
    main_with(Default::default());
}

/// The phone's folders, before anything reads them. Android's entry point does this from the
/// activity's paths (`android_main.rs`); iOS's from its own container, because the executable's
/// `main` is what iOS runs, on the main thread where winit's event loop must be made.
#[cfg(target_os = "ios")]
fn prepare_platform() {
    use std::path::PathBuf;
    let Some(container) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    // `Documents` is the part of the container the Files app shows (`UIFileSharingEnabled`
    // in the bundle's Info.plist): saves, resource packs and mods live there.
    let home = container.join("Documents");
    // What the OS may clear under memory pressure: the pack caches and the like.
    let cache = container.join("Library").join("Caches").join("horizon-cache");
    let _ = std::fs::create_dir_all(&home);
    let _ = std::fs::create_dir_all(&cache);
    // The folders players drop things into, ready in the Files app's Horizon folder.
    for dir in ["saves", "resourcepacks", "mods", "shaderpacks"] {
        let _ = std::fs::create_dir_all(home.join(dir));
    }
    // The prepared game files: a build can ship them in the bundle ("resources" beside the
    // executable, as on a desktop); a build without them downloads them into the game folder on
    // its first run, on the download screen (`horizon_client::setup`, `game/setup.rs`). Either
    // way the game reads one folder, the one named here.
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("resources")));
    let resources = match bundled {
        Some(dir) if dir.join("horizon-resources.json").is_file() => dir,
        _ => home.join("resources"),
    };
    // SAFETY: nothing else runs yet; the game reads these on this thread.
    unsafe {
        std::env::set_var("HORIZON_HOME", &home);
        std::env::set_var("HORIZON_CACHE", &cache);
        std::env::set_var("HORIZON_RESOURCES", &resources);
        if std::env::var_os("TMPDIR").is_none() {
            let tmp = container.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }
        // `spikes.txt` in the game folder (a number of milliseconds) turns on HORIZON_SPIKES:
        // frames slower than that log their timings, without a debug build (as on Android).
        if let Ok(text) = std::fs::read_to_string(home.join("spikes.txt"))
            && text.trim().parse::<f32>().is_ok()
        {
            std::env::set_var("HORIZON_SPIKES", text.trim());
        }
    }
    // The game folder is the working directory, as the launcher makes it on a desktop.
    let _ = std::env::set_current_dir(&home);
}

/// Start the game with trait mods in dispatch order.
pub fn main_with(mods: crate::mod_api::Mods) {
    #[cfg(target_os = "ios")]
    prepare_platform();
    // First: a trait injection into anything below (logging, the crash handler) calls its mod.
    if crate::mod_api::MODS.set(mods).is_err() {
        log::warn!("Trait mods already installed; ignoring the second list");
    }
    horizon_threads::configure_allocator();
    horizon_client::telemetry::mark_start();
    horizon_client::spans::startup::init(std::time::Instant::now());
    horizon_client::spans::trace::start_from_env();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let game_args = args.iter().take_while(|a| *a != "--dedicated");
    let help = game_args
        .clone()
        .any(|a| a == "--help" || a == "-h" || a == "--launch-api");
    // --home before the log opens in it.
    if let Some(dir) = game_args.clone().skip_while(|a| *a != "--home").nth(1) {
        logging::set_game_home(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
    }
    // The dedicated server logs in its own folder (--game-dir's), as vanilla's does; the game in its home.
    let (home, side, thread) = if args.iter().any(|a| a == "--dedicated") {
        let folder = game_args
            .clone()
            .skip_while(|a| *a != "--game-dir" && *a != "--gameDir")
            .nth(1)
            .and_then(|dir| std::path::absolute(dir).ok())
            .unwrap_or_else(|| ".".into());
        (folder, "server", "Server thread")
    } else {
        // --player-dir too (a game's, not a server's): a player of a shared game folder keeps
        // its log and crash reports in its own; crash telemetry stays with the home's consent.
        if let Some(dir) = game_args
            .clone()
            .skip_while(|a| *a != "--player-dir")
            .nth(1)
        {
            logging::set_player_dir(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
        }
        (logging::game_home(), "client", "Render thread")
    };
    // Before the log opens, so a mod's injection hears its first lines.
    logging::set_line_hook(crate::native_client_hooks::native_log_line);
    if !help {
        logging::init(
            logging::player_dir_override().unwrap_or(home.as_path()),
            thread,
        );
        // The log's first line names the build, so a bug report that carries the log says which one it was.
        log::info!("Horizon {}", crash::version());
        crash::install(home.clone(), side);
    }
    let renderers = crate::mod_api::mods()
        .iter()
        .filter_map(|m| m.render())
        .collect();
    if crate::gfx::native_render::install(renderers).is_err() {
        log::warn!("Trait renderers already installed; keeping the first list");
    }
    if horizon_world::native_load_hooks::install(
        horizon_world::native_load_hooks::NativeLoadHooks {
            loot_table_load: |id, table| {
                crate::mod_api::dispatch_loot_table_load(crate::mod_api::mods(), id, table);
            },
            modify_default_components: |item, components| {
                crate::mod_api::dispatch_modify_default_components(
                    crate::mod_api::mods(),
                    item,
                    components,
                );
            },
            loot_function: |kind, data, ctx, stack| {
                crate::mod_api::dispatch_loot_function(
                    crate::mod_api::mods(),
                    kind,
                    data,
                    ctx,
                    stack,
                );
            },
            loot_condition: |kind, data, ctx, holds| {
                crate::mod_api::dispatch_loot_condition(
                    crate::mod_api::mods(),
                    kind,
                    data,
                    ctx,
                    holds,
                );
            },
        },
    )
    .is_err()
    {
        log::warn!("Native data-load hooks already installed; keeping the first callbacks");
    }
    if horizon_world::worldgen::native_generate::install(
        horizon_world::worldgen::native_generate::NativeGenerateHooks {
            chunk_generate: |dimension, chunk, seed, flat, chunk_out, generate| {
                crate::mod_api::dispatch_chunk_generate(
                    crate::mod_api::mods(),
                    dimension,
                    chunk,
                    seed,
                    flat,
                    chunk_out,
                    generate,
                );
            },
            lod_column: |dimension, column, seed, flat, column_out| {
                crate::mod_api::dispatch_lod_column(
                    crate::mod_api::mods(),
                    dimension,
                    column,
                    seed,
                    flat,
                    column_out,
                );
            },
        },
    )
    .is_err()
    {
        log::warn!("Native generation hooks already installed; keeping the first callbacks");
    }
    if crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {
        menu_slot_rules: |menu, slot, stack, may_place, max_stack| {
            crate::mod_api::dispatch_menu_slot_rules(
                crate::mod_api::mods(),
                menu,
                slot,
                stack,
                may_place,
                max_stack,
            );
        },
        player_chat: |player, name, message, replies, cancel| {
            crate::mod_api::dispatch_player_chat(
                crate::mod_api::mods(),
                player,
                name,
                message,
                replies,
                cancel,
            );
        },
        sound_play: |sound, volume, pitch, cancel| {
            crate::mod_api::dispatch_sound_play(
                crate::mod_api::mods(),
                sound,
                volume,
                pitch,
                cancel,
            );
        },
        living_heal: |entity, amount, cancel| {
            crate::mod_api::dispatch_living_heal(crate::mod_api::mods(), entity, amount, cancel);
        },
        living_breathe: |entity, kind, can_breathe, consume, refill| {
            crate::mod_api::dispatch_living_breathe(
                crate::mod_api::mods(),
                entity,
                kind,
                can_breathe,
                consume,
                refill,
            );
        },
        living_drown: |entity, kind, is_drowning, damage_amount, bubble_count, cancel| {
            crate::mod_api::dispatch_living_drown(
                crate::mod_api::mods(),
                entity,
                kind,
                is_drowning,
                damage_amount,
                bubble_count,
                cancel,
            );
        },
        living_fall: |entity, kind, distance, multiplier, cancel| {
            crate::mod_api::dispatch_living_fall(
                crate::mod_api::mods(),
                entity,
                kind,
                distance,
                multiplier,
                cancel,
            );
        },
        living_jump: |entity, kind| {
            crate::mod_api::dispatch_living_jump(crate::mod_api::mods(), entity, kind);
        },
        register_goals: |mob| {
            crate::mod_api::dispatch_register_goals(crate::mod_api::mods(), mob);
        },
        player_pre_login: |player, name, address, defer| {
            crate::mod_api::dispatch_player_pre_login(
                crate::mod_api::mods(),
                player,
                name,
                address,
                defer,
            );
        },
        player_profile: |player, name, properties| {
            crate::mod_api::dispatch_player_profile(
                crate::mod_api::mods(),
                player,
                name,
                properties,
            );
        },
        ingredient: |kind, data, items| {
            crate::mod_api::dispatch_ingredient(crate::mod_api::mods(), kind, data, items);
        },
        ingredient_test: |kind, data, stack, accept| {
            crate::mod_api::dispatch_ingredient_test(
                crate::mod_api::mods(),
                kind,
                data,
                stack,
                accept,
            );
        },
        log_line: |level, thread, message, line| {
            crate::mod_api::dispatch_log_line(crate::mod_api::mods(), level, thread, message, line);
        },
        handshake: |address, host, intent, cancel, reason| {
            crate::mod_api::dispatch_handshake(
                crate::mod_api::mods(),
                address,
                host,
                intent,
                reason,
                cancel,
            );
        },
    })
    .is_err()
    {
        log::warn!("Native client hooks already installed; keeping the first callbacks");
    }
    if horizon_client::sound_engine::install_playback_hook(
        horizon_client::sound_playback::native_sound_play,
    )
    .is_err()
    {
        log::warn!("Native sound hook already installed; keeping the first callback");
    }
    raise_open_file_limit();
    if let Err(error) = game::run_with() {
        // A bad command line is the caller's mistake, not a crash: no report.
        let usage = error.is::<game::UsageError>();
        log::error!("Horizon: {error}");
        logging::flush();
        // The report's path is the last line on stderr (crash::SAVED_LINE).
        if !help && !usage {
            crash::report_error(&home, side, &*error);
        }
        // Started from Explorer, the console window closes with the game:
        // keep the message on screen until it is read.
        #[cfg(windows)]
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("Press Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(if usage {
            game::exit_code::USAGE
        } else {
            game::exit_code::ERROR
        });
    }
    logging::flush();
}

/// The JVM raises its open-file limit to the hard limit as it starts
/// (`-XX:+MaxFDLimit`, on by default), so a vanilla server may hold as many
/// sockets and region files as the host allows, not the soft default (often
/// 1024 in a container). The same here; macOS caps it at `OPEN_MAX`, as the
/// JVM does there.
fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let limit = getrlimit(Resource::Nofile);
        let hard = if cfg!(target_os = "macos") {
            Some(limit.maximum.map_or(10_240, |h| h.min(10_240)))
        } else {
            limit.maximum
        };
        let raised = match (limit.current, hard) {
            (Some(soft), Some(hard)) => hard > soft,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if raised {
            let to = Rlimit {
                current: hard,
                maximum: limit.maximum,
            };
            match setrlimit(Resource::Nofile, to) {
                Ok(()) => log::info!(
                    "Open file limit raised from {:?} to {hard:?}",
                    limit.current
                ),
                Err(e) => log::warn!("Open file limit stays at {:?}: {e}", limit.current),
            }
        }
    }
}
