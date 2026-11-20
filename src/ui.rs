// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2025-2026 dmth

//! AppKit UI: the floating search panel, its result list, and the About window.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor, NSControl,
    NSControlTextEditingDelegate, NSFloatingWindowLevel, NSFont, NSFontWeightRegular, NSImage,
    NSImageScaling, NSImageSymbolConfiguration, NSImageView, NSLayoutAttribute, NSLineBreakMode,
    NSPasteboard, NSPasteboardTypeString, NSScrollView, NSStackView, NSTableColumn, NSTableView,
    NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextField,
    NSTextFieldDelegate, NSTextView, NSUserInterfaceLayoutOrientation, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindow, NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
    NSAutoresizingMaskOptions,
};
use objc2_foundation::{
    ns_string, NSArray, NSEdgeInsets, NSIndexSet, NSInteger, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};

use crate::calc;
use crate::currency::{self, RateStore};
use crate::index::{self, AppEntry};
use crate::usage::Usage;

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 420.0;
const HEADER: f64 = 60.0;
const ROW_HEIGHT: f64 = 64.0;
const ICON_SIZE: f64 = 48.0;
const LOGO_SIZE: f64 = 44.0;
/// Left edge of the search pill: logo inset + logo + gap.
const PILL_X: f64 = 12.0 + LOGO_SIZE + 8.0;
const SITE_URL: &str = "https://falsetrue.io/projects/overlauncher";

#[derive(Clone)]
enum Item {
    /// A computed answer (calculation, conversion, status). Enter copies `copy`, if any.
    Answer { symbol: &'static str, title: String, subtitle: String, copy: Option<String> },
    App(AppEntry),
}

// ---------------------------------------------------------------------------
// Borderless window that can still take keyboard focus.
// ---------------------------------------------------------------------------

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "RLSearchWindow"]
    struct SearchWindow;

    impl SearchWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }
);

// ---------------------------------------------------------------------------
// Controller: owns state, acts as text-field / table / window delegate.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Ivars {
    window: OnceCell<Retained<SearchWindow>>,
    field: OnceCell<Retained<NSTextField>>,
    table: OnceCell<Retained<NSTableView>>,
    about: OnceCell<Retained<NSWindow>>,
    apps: RefCell<Vec<AppEntry>>,
    results: RefCell<Vec<Item>>,
    icons: RefCell<HashMap<PathBuf, Retained<NSImage>>>,
    usage: RefCell<Usage>,
    rates: RefCell<RateStore>,
    /// `--screenshots` mode: output dir and next step.
    shots: RefCell<Option<(PathBuf, usize)>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RLController"]
    #[ivars = Ivars]
    pub struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSControlTextEditingDelegate for Controller {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            self.refresh();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_do_command(&self, _control: &NSControl, _tv: &NSTextView, command: Sel) -> bool {
            self.handle_command(command)
        }
    }

    unsafe impl NSTextFieldDelegate for Controller {}

    unsafe impl NSTableViewDataSource for Controller {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.ivars().results.borrow().len() as NSInteger
        }
    }

    unsafe impl NSTableViewDelegate for Controller {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            _table: &NSTableView,
            _column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            let item = self.ivars().results.borrow().get(row as usize).cloned();
            item.map(|item| self.make_row_view(&item))
        }
    }

    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.hide();
        }
    }

    impl Controller {
        #[unsafe(method(rowClicked:))]
        fn row_clicked(&self, _sender: Option<&AnyObject>) {
            let row = self.table().clickedRow();
            if row >= 0 {
                self.activate(row as usize);
            }
        }

        #[unsafe(method(showAbout:))]
        fn show_about_action(&self, _sender: Option<&AnyObject>) {
            self.show_about();
        }

        #[unsafe(method(screenshotStep:))]
        fn screenshot_step_action(&self, _sender: Option<&AnyObject>) {
            self.screenshot_step();
        }

        #[unsafe(method(openRatesSite:))]
        fn open_rates_site(&self, _sender: Option<&AnyObject>) {
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(currency::ATTRIBUTION_URL)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(openSite:))]
        fn open_site(&self, _sender: Option<&AnyObject>) {
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(SITE_URL)) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }
    }
);

impl Controller {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let ivars = Ivars {
            usage: RefCell::new(Usage::load(Usage::default_file())),
            rates: RefCell::new(RateStore::load(RateStore::default_file())),
            ..Default::default()
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.build_search_window(mtm);
        this.reindex();
        let target = this.clone(); // the controller lives for the whole process
        currency::set_fetch_handler(move |result| target.on_rates_fetched(result));
        this
    }

    fn mtm(&self) -> MainThreadMarker {
        MainThreadMarker::from(self)
    }

    fn window(&self) -> &SearchWindow {
        self.ivars().window.get().expect("window built in new()")
    }

    fn field(&self) -> &NSTextField {
        self.ivars().field.get().expect("field built in new()")
    }

    fn table(&self) -> &NSTableView {
        self.ivars().table.get().expect("table built in new()")
    }

    // ----- show / hide ------------------------------------------------------

    pub fn toggle(&self) {
        let window = self.window();
        if window.isVisible() && window.isKeyWindow() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn show(&self) {
        self.reindex(); // cheap; picks up newly installed apps
        let window = self.window();
        self.field().setStringValue(ns_string!(""));
        self.refresh();
        window.center();
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
        window.makeFirstResponder(Some(self.field()));
    }

    pub fn hide(&self) {
        let window = self.window();
        if !window.isVisible() {
            return;
        }
        window.orderOut(None);
        let about_visible = self.ivars().about.get().is_some_and(|w| w.isVisible());
        if !about_visible {
            // Hand focus back to whichever app was frontmost before us.
            NSApplication::sharedApplication(self.mtm()).hide(None);
        }
    }

    // ----- search -----------------------------------------------------------

    fn reindex(&self) {
        let apps = index::scan(index::APP_DIRECTORIES, bundle_display_name);
        *self.ivars().apps.borrow_mut() = apps;
    }

    fn refresh(&self) {
        let query = self.field().stringValue().to_string();
        let mut results = Vec::new();
        if let Some(c) = calc::calculate(&query) {
            results.push(Item::Answer {
                symbol: "function",
                title: c.display,
                subtitle: "Press Enter to copy result".into(),
                copy: Some(c.value),
            });
        }
        results.extend(self.currency_item(&query));
        {
            let apps = self.ivars().apps.borrow();
            let usage = self.ivars().usage.borrow();
            let hits = index::search(&apps, &query, |app| usage.get(&app.path));
            results.extend(hits.into_iter().cloned().map(Item::App));
        }
        *self.ivars().results.borrow_mut() = results;
        self.table().reloadData();
        self.select(0);
    }

    /// Row for `100 USD to EUR`-style queries; kicks off a rate fetch when needed.
    fn currency_item(&self, query: &str) -> Option<Item> {
        let q = currency::parse_query(query)?;
        let now = currency::now();
        {
            let mut store = self.ivars().rates.borrow_mut();
            if store.needs_refresh(now) {
                store.fetching = true;
                currency::spawn_fetch();
            }
        }
        let store = self.ivars().rates.borrow();
        let amount = calc::format_number(q.amount);
        let answer = |title: String, subtitle: String, copy: Option<String>| Item::Answer {
            symbol: "dollarsign.arrow.circlepath",
            title,
            subtitle,
            copy,
        };
        Some(match (&store.rates, &store.last_error) {
            (Some(rates), _) => match rates.convert(&q) {
                Some((value, rate)) => {
                    let (display, plain) = currency::format_money(value);
                    let age = currency::format_age(now.saturating_sub(rates.updated));
                    answer(
                        format!("{amount} {} = {display} {}", q.from, q.to),
                        format!("1 {} = {} {} · rates updated {age} · Enter to copy", q.from, currency::format_rate(rate), q.to),
                        Some(plain),
                    )
                }
                None => {
                    let unknown = if rates.per_usd.contains_key(&q.from) { &q.to } else { &q.from };
                    answer(
                        format!("Unknown currency: {unknown}"),
                        "Use ISO codes like USD, EUR, GBP, JPY".into(),
                        None,
                    )
                }
            },
            (None, Some((_, err))) if !store.fetching => answer(
                "Exchange rates unavailable".into(),
                format!("Couldn't download rates ({err}). Check your connection."),
                None,
            ),
            (None, _) => answer(
                format!("{amount} {} → {}", q.from, q.to),
                "Fetching exchange rates…".into(),
                None,
            ),
        })
    }

    fn on_rates_fetched(&self, result: Result<String, String>) {
        self.ivars().rates.borrow_mut().finish(result, currency::now());
        if self.window().isVisible() {
            self.refresh();
        }
    }

    fn select(&self, row: usize) {
        let table = self.table();
        let count = self.ivars().results.borrow().len();
        if count == 0 {
            unsafe { table.deselectAll(None) };
            return;
        }
        let row = row.min(count - 1);
        table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row), false);
        table.scrollRowToVisible(row as NSInteger);
    }

    fn move_selection(&self, delta: isize) {
        let current = self.table().selectedRow().max(0);
        self.select((current + delta).max(0) as usize);
    }

    fn handle_command(&self, command: Sel) -> bool {
        if command == sel!(moveDown:) {
            self.move_selection(1);
        } else if command == sel!(moveUp:) {
            self.move_selection(-1);
        } else if command == sel!(insertNewline:) {
            let row = self.table().selectedRow();
            if row >= 0 {
                self.activate(row as usize);
            }
        } else if command == sel!(cancelOperation:) {
            self.hide();
        } else {
            return false;
        }
        true
    }

    fn activate(&self, row: usize) {
        let Some(item) = self.ivars().results.borrow().get(row).cloned() else { return };
        match item {
            Item::App(app) => {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&app.path.to_string_lossy()));
                if NSWorkspace::sharedWorkspace().openURL(&url) {
                    if let Err(err) = self.ivars().usage.borrow_mut().record(&app.path) {
                        eprintln!("rolauncher: failed to save usage stats: {err}");
                    }
                } else {
                    eprintln!("rolauncher: failed to open {}", app.path.display());
                }
            }
            Item::Answer { copy: Some(value), .. } => {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                pasteboard.setString_forType(&NSString::from_str(&value), unsafe { NSPasteboardTypeString });
            }
            Item::Answer { copy: None, .. } => return, // status row; nothing to do
        }
        self.hide();
    }

    // ----- view construction ------------------------------------------------

    fn build_search_window(&self, mtm: MainThreadMarker) {
        let frame = rect(0.0, 0.0, WIDTH, HEIGHT);
        let window: Retained<SearchWindow> = unsafe {
            msg_send![
                SearchWindow::alloc(mtm),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::FullSizeContentView,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setLevel(NSFloatingWindowLevel);
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::MoveToActiveSpace | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.setDelegate(Some(ProtocolObject::from_ref(self)));

        // Translucent rounded background.
        let background = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        background.setMaterial(NSVisualEffectMaterial::Popover);
        background.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        background.setState(NSVisualEffectState::Active);
        background.setWantsLayer(true);
        if let Some(layer) = background.layer() {
            layer.setCornerRadius(12.0);
            layer.setMasksToBounds(true);
        }
        window.setContentView(Some(&background));

        // Header: magnifying glass, search field, settings and quit buttons.
        let header_mid = HEIGHT - HEADER / 2.0;

        // Lighter header band, with an even lighter rounded pill behind the field.
        let header_bg = filled_box(rect(0.0, HEIGHT - HEADER, WIDTH, HEADER), 0.10, 0.0, mtm);
        background.addSubview(&header_bg);
        let pill = filled_box(rect(PILL_X, header_mid - 20.0, WIDTH - PILL_X - 94.0, 40.0), 0.14, 10.0, mtm);
        background.addSubview(&pill);

        // App logo, left of the magnifying glass.
        if let Some(icon) = NSApplication::sharedApplication(mtm).applicationIconImage() {
            let logo = NSImageView::imageViewWithImage(&icon, mtm);
            logo.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            logo.setFrame(rect(12.0, header_mid - LOGO_SIZE / 2.0, LOGO_SIZE, LOGO_SIZE));
            background.addSubview(&logo);
        }

        let glass = symbol_view("magnifyingglass", 18.0, mtm);
        glass.setFrame(rect(PILL_X + 8.0, header_mid - 12.0, 24.0, 24.0));
        glass.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        background.addSubview(&glass);

        let field = NSTextField::new(mtm);
        field.setFrame(rect(PILL_X + 40.0, header_mid - 14.0, WIDTH - PILL_X - 40.0 - 96.0, 28.0));
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(objc2_app_kit::NSFocusRingType::None);
        field.setFont(Some(&NSFont::systemFontOfSize(22.0)));
        field.setPlaceholderString(Some(ns_string!("Search applications...")));
        if let Some(cell) = field.cell() { cell.setUsesSingleLineMode(true) }
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(self))) };
        background.addSubview(&field);

        let settings = icon_button("gearshape.fill", "Settings", mtm);
        settings.setFrame(rect(WIDTH - 84.0, header_mid - 14.0, 28.0, 28.0));
        unsafe {
            settings.setTarget(Some(self));
            settings.setAction(Some(sel!(showAbout:)));
        }
        background.addSubview(&settings);

        let quit = icon_button("power", "Quit RoLauncher", mtm);
        quit.setFrame(rect(WIDTH - 48.0, header_mid - 14.0, 28.0, 28.0));
        unsafe {
            quit.setTarget(Some(&NSApplication::sharedApplication(mtm)));
            quit.setAction(Some(sel!(terminate:)));
        }
        background.addSubview(&quit);

        let separator = NSBox::initWithFrame(NSBox::alloc(mtm), rect(0.0, HEIGHT - HEADER - 1.0, WIDTH, 1.0));
        separator.setBoxType(NSBoxType::Separator);
        background.addSubview(&separator);

        // Results list.
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0.0, 0.0, WIDTH, HEIGHT - HEADER - 1.0));
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);

        let table = NSTableView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("main"));
        column.setWidth(WIDTH - 20.0);
        table.addTableColumn(&column);
        table.setHeaderView(None);
        table.setRowHeight(ROW_HEIGHT);
        table.setStyle(NSTableViewStyle::Inset);
        table.setBackgroundColor(&NSColor::clearColor());
        table.setRefusesFirstResponder(true);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(self)));
            table.setDelegate(Some(ProtocolObject::from_ref(self)));
            table.setTarget(Some(self));
            table.setAction(Some(sel!(rowClicked:)));
        }
        scroll.setDocumentView(Some(&table));
        background.addSubview(&scroll);

        let _ = self.ivars().window.set(window);
        let _ = self.ivars().field.set(field);
        let _ = self.ivars().table.set(table);
    }

    fn make_row_view(&self, item: &Item) -> Retained<NSView> {
        let mtm = self.mtm();
        let width = WIDTH - 40.0;
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, width, ROW_HEIGHT));

        let (icon, title, subtitle) = match item {
            Item::Answer { symbol, title, subtitle, .. } => {
                let icon = symbol_view(symbol, 32.0, mtm);
                icon.setContentTintColor(Some(&NSColor::controlAccentColor()));
                (icon, title.clone(), subtitle.clone())
            }
            Item::App(app) => {
                let image = self.icon_for(&app.path);
                (NSImageView::imageViewWithImage(&image, mtm), app.name.clone(), app.path.display().to_string())
            }
        };
        icon.setFrame(rect(8.0, (ROW_HEIGHT - ICON_SIZE) / 2.0, ICON_SIZE, ICON_SIZE));
        let text_x = 8.0 + ICON_SIZE + 12.0;
        let text_bottom = (ROW_HEIGHT - 38.0) / 2.0; // 16pt subtitle + 2pt gap + 20pt title
        icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        view.addSubview(&icon);

        let title_label = label(&title, NSFont::systemFontOfSize(16.0), mtm);
        title_label.setFrame(rect(text_x, text_bottom + 18.0, width - text_x - 10.0, 20.0));
        title_label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        view.addSubview(&title_label);

        let sub_label = label(&subtitle, NSFont::systemFontOfSize(11.0), mtm);
        sub_label.setFrame(rect(text_x, text_bottom, width - text_x - 10.0, 16.0));
        sub_label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        sub_label.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        view.addSubview(&sub_label);

        for v in [&*title_label, &*sub_label] {
            v.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        }
        view
    }

    fn icon_for(&self, path: &Path) -> Retained<NSImage> {
        self.ivars()
            .icons
            .borrow_mut()
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()))
            })
            .clone()
    }

    // ----- About window -----------------------------------------------------

    /// Walks through a few representative states, saving a PNG of each to `dir`, then quits.
    pub fn start_screenshots(&self, dir: PathBuf) {
        *self.ivars().shots.borrow_mut() = Some((dir, 0));
        self.screenshot_step();
    }

    fn screenshot_step(&self) {
        const STEPS: [(&str, Option<&str>); 5] = [
            ("launcher.png", Some("")),
            ("search.png", Some("cal")),
            ("calculator.png", Some("(2+3)*4/2")),
            ("currency.png", Some("100 USD to EUR")),
            ("about.png", None),
        ];
        let Some((dir, step)) = self.ivars().shots.borrow().clone() else { return };

        // Capture the state set up by the previous step.
        if let Some((name, query)) = step.checked_sub(1).map(|i| STEPS[i]) {
            let window: &NSWindow = match query {
                Some(_) => self.window(),
                None => self.ivars().about.get().expect("about shown"),
            };
            match crate::screenshot::capture(window, &dir.join(name)) {
                Ok(()) => println!("saved {}", dir.join(name).display()),
                Err(err) => eprintln!("rolauncher: {err}"),
            }
        }

        let Some(&(_, query)) = STEPS.get(step) else {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
            return;
        };
        match query {
            Some(q) => {
                if !self.window().isVisible() {
                    self.show();
                }
                self.field().setStringValue(&NSString::from_str(q));
                self.refresh();
                // Put the caret at the end instead of selecting the text.
                if let Some(editor) = self.field().currentEditor() {
                    editor.setSelectedRange(objc2_foundation::NSRange::new(q.encode_utf16().count(), 0));
                }
            }
            None => self.show_about(),
        }
        *self.ivars().shots.borrow_mut() = Some((dir, step + 1));
        // Let AppKit render (and the window server composite) before capturing;
        // give a possible exchange-rate download extra time.
        let delay = if query.is_some_and(|q| currency::parse_query(q).is_some()) { 3.0f64 } else { 0.8 };
        unsafe {
            let _: () = msg_send![self, performSelector: sel!(screenshotStep:), withObject: std::ptr::null::<AnyObject>(), afterDelay: delay];
        }
    }

    pub fn show_about(&self) {
        let mtm = self.mtm();
        let window = self.ivars().about.get_or_init(|| build_about_window(self, mtm));
        window.center();
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

fn build_about_window(controller: &Controller, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, 460.0, 560.0),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(ns_string!("About RoLauncher"));
    window.setLevel(NSFloatingWindowLevel);

    let mut views: Vec<Retained<NSView>> = Vec::new();

    if let Some(icon) = NSApplication::sharedApplication(mtm).applicationIconImage() {
        let image_view = NSImageView::imageViewWithImage(&icon, mtm);
        image_view.widthAnchor().constraintEqualToConstant(64.0).setActive(true);
        image_view.heightAnchor().constraintEqualToConstant(64.0).setActive(true);
        views.push(image_view.into_super().into_super());
    }
    views.push(view(label("RoLauncher", NSFont::boldSystemFontOfSize(24.0), mtm)));
    views.push(view(secondary(
        &format!("Lightweight macOS app launcher. v{}", env!("CARGO_PKG_VERSION")),
        11.0,
        mtm,
    )));
    views.push(view(heading("How to Use", mtm)));
    for (keys, text) in [
        ("⌘ Space", "Open launcher"),
        ("↑ ↓", "Navigate through apps"),
        ("Enter", "Launch selected app"),
        ("Esc", "Close launcher (hides, doesn't quit)"),
        ("X+Y", "Type a math expression to get the result; Enter copies it"),
        ("100 USD to EUR", "Convert currencies at current exchange rates; Enter copies the result"),
        ("⏻", "Quit with the power icon in the top-right corner"),
    ] {
        let key = label(keys, NSFont::monospacedSystemFontOfSize_weight(13.0, unsafe { NSFontWeightRegular }), mtm);
        key.widthAnchor().constraintEqualToConstant(124.0).setActive(true);
        let desc = secondary(text, 13.0, mtm);
        desc.setPreferredMaxLayoutWidth(276.0);
        let row = NSStackView::stackViewWithViews(&NSArray::from_slice(&[view(key).as_ref(), view(desc).as_ref()]), mtm);
        row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        row.setAlignment(NSLayoutAttribute::FirstBaseline);
        row.setSpacing(8.0);
        views.push(row.into_super());
    }
    views.push(view(heading("About", mtm)));
    let blurb = secondary(
        "A fast, light, native macOS application launcher. RoLauncher gives instant access to \
         your applications with search and keyboard navigation. No telemetry, no ads, just pure \
         functionality.",
        13.0,
        mtm,
    );
    blurb.setPreferredMaxLayoutWidth(410.0);
    views.push(view(blurb));
    views.push(view(secondary("RoLauncher © 2025-2026 dmth. Free software under the GNU GPL v3.", 11.0, mtm)));

    let link = unsafe {
        NSButton::buttonWithTitle_target_action(ns_string!("RoLauncher site"), Some(controller), Some(sel!(openSite:)), mtm)
    };
    link.setBordered(false);
    link.setContentTintColor(Some(&NSColor::linkColor()));
    views.push(link.into_super().into_super());

    // Attribution requested by the free exchange-rate API's terms.
    let rates_link = unsafe {
        NSButton::buttonWithTitle_target_action(ns_string!("Rates by Exchange Rate API"), Some(controller), Some(sel!(openRatesSite:)), mtm)
    };
    rates_link.setBordered(false);
    rates_link.setContentTintColor(Some(&NSColor::linkColor()));
    views.push(rates_link.into_super().into_super());

    let close = unsafe {
        NSButton::buttonWithTitle_target_action(ns_string!("Close"), Some(&window), Some(sel!(performClose:)), mtm)
    };
    close.setKeyEquivalent(ns_string!("\r"));
    views.push(close.into_super().into_super());

    let refs: Vec<&NSView> = views.iter().map(|v| v.as_ref()).collect();
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&refs), mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setAlignment(NSLayoutAttribute::Leading);
    stack.setSpacing(10.0);
    stack.setEdgeInsets(NSEdgeInsets { top: 24.0, left: 24.0, bottom: 24.0, right: 24.0 });
    window.setContentView(Some(&stack));
    window
}

// ---------------------------------------------------------------------------
// Small view helpers.
// ---------------------------------------------------------------------------

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn view(field: Retained<NSTextField>) -> Retained<NSView> {
    field.into_super().into_super()
}

fn label(text: &str, font: Retained<NSFont>, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&font));
    label
}

fn secondary(text: &str, size: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    label.setSelectable(false);
    label
}

fn heading(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    
    label(text, NSFont::boldSystemFontOfSize(15.0), mtm)
}

/// Borderless rounded rectangle filled with translucent white, which lightens
/// whatever is behind it in both light and dark appearance.
fn filled_box(frame: NSRect, white_alpha: f64, corner_radius: f64, mtm: MainThreadMarker) -> Retained<NSBox> {
    let b = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(objc2_app_kit::NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b.setCornerRadius(corner_radius);
    b.setFillColor(&NSColor::colorWithWhite_alpha(1.0, white_alpha));
    b
}

fn symbol_view(name: &str, point_size: f64, mtm: MainThreadMarker) -> Retained<NSImageView> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
        .unwrap_or_default();
    let view = NSImageView::imageViewWithImage(&image, mtm);
    view.setSymbolConfiguration(Some(&NSImageSymbolConfiguration::configurationWithPointSize_weight(
        point_size,
        unsafe { NSFontWeightRegular },
    )));
    view
}

fn icon_button(symbol: &str, tooltip: &str, mtm: MainThreadMarker) -> Retained<NSButton> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        Some(&NSString::from_str(tooltip)),
    )
    .unwrap_or_default();
    let button = unsafe { NSButton::buttonWithImage_target_action(&image, None, None, mtm) };
    button.setBordered(false);
    button.setSymbolConfiguration(Some(&NSImageSymbolConfiguration::configurationWithPointSize_weight(
        16.0,
        unsafe { NSFontWeightRegular },
    )));
    button.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
    button.setToolTip(Some(&NSString::from_str(tooltip)));
    button
}

/// `CFBundleDisplayName` from the bundle's Info.plist (localized), if present.
fn bundle_display_name(path: &Path) -> Option<String> {
    let bundle = objc2_foundation::NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?))?;
    let value = bundle.objectForInfoDictionaryKey(ns_string!("CFBundleDisplayName"))?;
    Some(value.downcast::<NSString>().ok()?.to_string())
}
