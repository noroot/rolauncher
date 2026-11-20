// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! RoLauncher — a lightweight macOS application launcher (Rust/AppKit port).

mod calc;
mod currency;
mod hotkey;
mod index;
mod screenshot;
mod ui;
mod usage;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

fn main() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    migrate_data_dir();
    let app = NSApplication::sharedApplication(mtm);
    // Background app: no Dock icon (also set via LSUIElement in the bundle's Info.plist).
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let controller = ui::Controller::new(mtm);
    app.setMainMenu(Some(&build_main_menu(&controller, mtm)));

    let target = controller.clone();
    if let Err(status) = hotkey::register(hotkey::KEY_SPACE, hotkey::MOD_CMD, move || target.toggle()) {
        eprintln!("rolauncher: failed to register ⌘Space hotkey (OSStatus {status})");
    }

    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--screenshots") {
        // Dev aid: `--screenshots <dir>` saves PNGs of the UI and quits.
        let dir = std::path::PathBuf::from(args.get(i + 1).map_or("screenshots", String::as_str));
        std::fs::create_dir_all(&dir).expect("create screenshot dir");
        controller.start_screenshots(dir);
    } else if args.iter().any(|a| a == "--show") {
        controller.show();
    }
    app.run();
}

/// One-time move of data saved under the app's former name (OverLauncher) to RoLauncher's folder.
fn migrate_data_dir() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let support = std::path::Path::new(&home).join("Library/Application Support");
    let (old, new) = (support.join("OverLauncher"), support.join("RoLauncher"));
    if old.is_dir() && !new.exists() && let Err(err) = std::fs::rename(&old, &new) {
        eprintln!("rolauncher: could not migrate {}: {err}", old.display());
    }
}

/// The menu bar is never visible for an accessory app, but its key equivalents
/// still drive ⌘C / ⌘V / ⌘A / ⌘Q / ⌘, inside the search field.
fn build_main_menu(controller: &ui::Controller, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::new(mtm);

    let app_menu = submenu(&main, "RoLauncher", mtm);
    let about = item("About RoLauncher", sel!(showAbout:), ",", mtm);
    unsafe { about.setTarget(Some(controller as &AnyObject)) };
    app_menu.addItem(&about);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    app_menu.addItem(&item("Quit RoLauncher", sel!(terminate:), "q", mtm));

    let edit = submenu(&main, "Edit", mtm);
    for (title, action, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        edit.addItem(&item(title, action, key, mtm));
    }
    main
}

fn submenu(parent: &NSMenu, title: &str, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let title = NSString::from_str(title);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
    let holder = NSMenuItem::new(mtm);
    holder.setSubmenu(Some(&menu));
    parent.addItem(&holder);
    menu
}

fn item(title: &str, action: Sel, key: &str, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        )
    }
}
