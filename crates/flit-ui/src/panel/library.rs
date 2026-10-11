//! The Library (PACK 2 §3): SONGS, ALBUMS, ORGS, INVITES. UI words: a Name
//! is a Studio, a being a member, an organization an org. Everything here
//! reads `Shared.library` / `Shared.albums` / `Shared.org_members` and
//! writes requests; the helper is the truth.

use crate::client::Shared;
use crate::theme as t;
use flit_core::{Access, AlbumAction, AlbumId, ContainerKind, SongId};
use flit_ipc::*;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Songs,
    Albums,
    Orgs,
    Invites,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum OrgTab {
    #[default]
    Songs,
    Albums,
    Members,
    Settings,
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    fetched: bool,
    org: Option<SongId>,
    org_tab: OrgTab,
    album: Option<(AlbumScope, AlbumId)>,
    // forms
    new_org: String,
    new_album: String,
    new_album_org: Option<SongId>,
    invite_id: String,
    org_label: String,
    song_name: String,
    song_dir: String,
    song_org: Option<SongId>,
    song_access: usize,
    add_song: String,
    cover_path: String,
    /// Decoded covers by hash (textures live on the context).
    textures: std::collections::HashMap<flit_core::Hash, egui::TextureHandle>,
}

impl State {
    /// Open the Library on a tab (the rail).
    pub fn open(&mut self, tab: Tab, shared: &Shared) {
        self.tab = tab;
        self.refresh(shared);
    }

    pub fn refresh(&self, shared: &Shared) {
        shared.act(Request::Library);
        shared.act(Request::Albums);
        if let Some(o) = self.org {
            shared.act(Request::OrgMembers { org: o });
        }
    }
}

fn access_word(a: Option<Access>) -> &'static str {
    match a {
        Some(Access::Org) => "org",
        Some(Access::Private) => "private",
        None => "",
    }
}

fn daw_word(host: &str) -> &str {
    match host {
        "fl" => "FL Studio",
        "fake" => "the fake DAW",
        "" | "plugin" => "the DAW",
        other => other,
    }
}

fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    if text.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(text).font(t::text(t::S11)).color(color));
}

pub fn show(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared) {
    if !st.fetched {
        st.fetched = true;
        st.refresh(shared);
    }
    let lib = shared.library.lock().clone();
    let Some(lib) = lib else {
        t::muted(ui, "Loading the Library");
        return;
    };
    ui.horizontal(|ui| {
        let tabs = ["Songs", "Albums", "Orgs", if lib.invites.is_empty() { "Invites" } else { "Invites •" }];
        let cur = match st.tab {
            Tab::Songs => 0,
            Tab::Albums => 1,
            Tab::Orgs => 2,
            Tab::Invites => 3,
        };
        let idx = t::segmented(ui, cur, &tabs);
        if idx != cur {
            st.tab = [Tab::Songs, Tab::Albums, Tab::Orgs, Tab::Invites][idx];
            st.org = None;
            st.album = None;
            st.refresh(shared);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if t::quiet(ui, "Refresh").clicked() {
                st.refresh(shared);
            }
        });
    });
    ui.add_space(t::GAP);
    match st.tab {
        Tab::Songs => songs(ui, st, s, shared, &lib),
        Tab::Albums => albums(ui, st, s, shared, &lib),
        Tab::Orgs => orgs(ui, st, s, shared, &lib),
        Tab::Invites => invites(ui, shared, &lib),
    }
}

// ----- SONGS -----

fn songs(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared, lib: &LibraryView) {
    t::card(ui, |ui| {
        t::section_label(ui, "New song");
        ui.horizontal(|ui| {
            t::muted(ui, "Under");
            let label = st.song_org.and_then(|o| lib.orgs.iter().find(|r| r.org == o)).map(|r| r.name.clone()).unwrap_or_else(|| "Me".into());
            egui::ComboBox::from_id_salt("new-song-under").selected_text(egui::RichText::new(label).font(t::text(t::S12))).show_ui(ui, |ui| {
                if ui.selectable_label(st.song_org.is_none(), "Me").clicked() {
                    st.song_org = None;
                }
                for o in lib.orgs.iter().filter(|o| o.member && !o.archived) {
                    if ui.selectable_label(st.song_org == Some(o.org), &o.name).clicked() {
                        st.song_org = Some(o.org);
                        st.song_access = if o.defaults.access == Access::Org { 0 } else { 1 };
                    }
                }
            });
            if st.song_org.is_some() {
                t::muted(ui, "Access");
                st.song_access = t::segmented(ui, st.song_access, &["Org", "Private"]);
            }
        });
        t::field(ui, "Name", &mut st.song_name, "My song");
        t::field(ui, "Folder", &mut st.song_dir, "C:\\Music\\MySong");
        ui.horizontal(|ui| {
            let ready = !st.song_name.trim().is_empty() && !st.song_dir.trim().is_empty();
            if t::primary(ui, "Create").clicked() && ready {
                let access = st.song_org.map(|_| if st.song_access == 0 { Access::Org } else { Access::Private });
                shared.act(Request::NewSongIn { name: st.song_name.trim().into(), dir: st.song_dir.trim().into(), org: st.song_org, access });
                st.song_name.clear();
            }
            if t::button(ui, &format!("Create and open {}", daw_word(&s.host))).on_hover_text("Makes the song, then starts the DAW: save the project into the song's folder").clicked() && ready {
                let access = st.song_org.map(|_| if st.song_access == 0 { Access::Org } else { Access::Private });
                let dir: std::path::PathBuf = st.song_dir.trim().into();
                shared.act(Request::NewSongIn { name: st.song_name.trim().into(), dir: dir.clone(), org: st.song_org, access });
                shared.act(Request::OpenInDaw { dir });
                st.song_name.clear();
            }
        });
    });
    ui.add_space(t::GAP);
    if let Some(song) = &s.song {
        t::card(ui, |ui| {
            t::section_label(ui, "Org & access");
            ui.horizontal(|ui| {
                t::strong(ui, &song.name);
                match &lib.song_org {
                    Some((_, name)) => badge(ui, &format!("org {name}"), t::ACCENT),
                    None => t::muted(ui, "no org"),
                }
                badge(ui, lib.access.word(), t::MUTED);
            });
            ui.horizontal(|ui| {
                t::muted(ui, "Org");
                let current = lib.song_org.as_ref().map(|(o, _)| *o);
                let label = lib.song_org.as_ref().map(|(_, n)| n.clone()).unwrap_or_else(|| "none".into());
                egui::ComboBox::from_id_salt("song-org").selected_text(egui::RichText::new(label).font(t::text(t::S12))).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "none").clicked() && current.is_some() {
                        shared.act(Request::SetSongOrg { org: None, access: Access::Private });
                    }
                    for o in lib.orgs.iter().filter(|o| o.member && !o.archived) {
                        if ui.selectable_label(current == Some(o.org), &o.name).clicked() && current != Some(o.org) {
                            shared.act(Request::SetSongOrg { org: Some(o.org), access: o.defaults.access });
                        }
                    }
                });
                if let Some(o) = current {
                    let cur = if lib.access == Access::Org { 0 } else { 1 };
                    let idx = t::segmented(ui, cur, &["Org", "Private"]);
                    if idx != cur {
                        shared.act(Request::SetSongOrg { org: Some(o), access: if idx == 0 { Access::Org } else { Access::Private } });
                    }
                }
            });
            let who = match (lib.song_org.as_ref(), lib.access) {
                (Some((_, n)), Access::Org) => format!("Who can act here: the song's crew and every member of {n}."),
                _ => "Who can act here: the song's crew.".to_string(),
            };
            t::small(ui, &who);
            // albums this song is in
            let albums = shared.albums.lock().clone();
            let in_albums: Vec<&AlbumRow> = albums.iter().filter(|a| a.songs.iter().any(|x| x.song == song.id)).collect();
            ui.horizontal_wrapped(|ui| {
                t::muted(ui, "Albums:");
                for a in &in_albums {
                    badge(ui, &format!("{} ({})", a.name, a.scope_name), t::TEXT);
                }
                if in_albums.is_empty() {
                    t::small(ui, "none");
                }
                egui::ComboBox::from_id_salt("add-to-album").selected_text(egui::RichText::new("+ Add to album").font(t::text(t::S12))).show_ui(ui, |ui| {
                    for a in albums.iter().filter(|a| !a.archived && !a.songs.iter().any(|x| x.song == song.id)) {
                        if ui.selectable_label(false, format!("{} ({})", a.name, a.scope_name)).clicked() {
                            shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::AddSong { song: song.id, after: None } });
                        }
                    }
                });
            });
        });
        ui.add_space(t::GAP);
    }
    t::section_label(ui, "All songs");
    if lib.songs.is_empty() {
        t::muted(ui, "No songs yet.");
    }
    for row in &lib.songs {
        ui.horizontal(|ui| {
            let open = s.song.as_ref().map(|x| x.dir == row.dir).unwrap_or(false);
            if open {
                t::dot(ui, t::OK);
            }
            if t::quiet(ui, &row.name).clicked() {
                shared.act(Request::OpenSong { dir: row.dir.clone() });
            }
            if t::button(ui, &format!("Open in {}", daw_word(&s.host))).on_hover_text("Open the song here and its project in the DAW").clicked() {
                shared.act(Request::OpenInDaw { dir: row.dir.clone() });
            }
            if let Some(o) = row.org.and_then(|o| lib.orgs.iter().find(|r| r.org == o)) {
                badge(ui, &o.name, t::ACCENT);
            }
            badge(ui, access_word(row.access), t::MUTED);
            ui.label(egui::RichText::new(row.dir.display().to_string()).font(t::code(t::S11)).color(t::FAINT));
        });
    }
}

// ----- ALBUMS -----

fn albums(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared, lib: &LibraryView) {
    let albums = shared.albums.lock().clone();
    if let Some((scope, id)) = st.album.clone() {
        if let Some(a) = albums.iter().find(|a| a.scope == scope && a.album == id) {
            album_page(ui, st, s, shared, lib, a);
            return;
        }
        st.album = None;
    }
    t::card(ui, |ui| {
        t::section_label(ui, "New album");
        ui.horizontal(|ui| {
            t::muted(ui, "For");
            let label = st.new_album_org.and_then(|o| lib.orgs.iter().find(|r| r.org == o)).map(|r| r.name.clone()).unwrap_or_else(|| "Me".into());
            egui::ComboBox::from_id_salt("new-album-for").selected_text(egui::RichText::new(label).font(t::text(t::S12))).show_ui(ui, |ui| {
                if ui.selectable_label(st.new_album_org.is_none(), "Me").clicked() {
                    st.new_album_org = None;
                }
                for o in lib.orgs.iter().filter(|o| o.member && !o.archived) {
                    if ui.selectable_label(st.new_album_org == Some(o.org), &o.name).clicked() {
                        st.new_album_org = Some(o.org);
                    }
                }
            });
            ui.add(egui::TextEdit::singleline(&mut st.new_album).hint_text("Album name").desired_width(200.0).font(t::text(t::S12)));
            if t::primary(ui, "New album").clicked() && !st.new_album.trim().is_empty() {
                let scope = st.new_album_org.map(AlbumScope::Org).unwrap_or(AlbumScope::Personal);
                shared.act(Request::NewAlbum { scope, name: st.new_album.trim().into() });
                st.new_album.clear();
            }
        });
    });
    ui.add_space(t::GAP);
    let mut groups: Vec<(String, Vec<&AlbumRow>)> = Vec::new();
    for a in &albums {
        match groups.iter_mut().find(|(n, _)| *n == a.scope_name) {
            Some((_, v)) => v.push(a),
            None => groups.push((a.scope_name.clone(), vec![a])),
        }
    }
    if groups.is_empty() {
        t::muted(ui, "No albums yet.");
    }
    for (name, rows) in groups {
        t::section_label(ui, &name);
        for a in rows {
            ui.horizontal(|ui| {
                if t::quiet(ui, &a.name).clicked() {
                    st.album = Some((a.scope.clone(), a.album));
                    shared.act(Request::AlbumTimeline { scope: a.scope.clone(), album: a.album });
                }
                t::small(ui, &format!("{} songs", a.songs.len()));
                if a.archived {
                    badge(ui, "archived", t::FAINT);
                }
            });
        }
    }
}

/// The album's cover as a texture: asked from the helper once, decoded once.
fn cover_texture(ui: &mut egui::Ui, st: &mut State, shared: &Shared, a: &AlbumRow) -> Option<egui::TextureHandle> {
    let hash = a.cover?;
    if let Some(tex) = st.textures.get(&hash) {
        return Some(tex.clone());
    }
    let bytes = shared.covers.lock().get(&hash).cloned();
    match bytes {
        Some(bytes) => {
            let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
            let tex = ui.ctx().load_texture(format!("cover-{}", hash.short()), ci, egui::TextureOptions::LINEAR);
            st.textures.insert(hash, tex.clone());
            Some(tex)
        }
        None => {
            if shared.covers_wanted.lock().insert(hash) {
                shared.act(Request::ContainerBlob { scope: a.scope.clone(), hash });
            }
            None
        }
    }
}

fn album_page(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared, lib: &LibraryView, a: &AlbumRow) {
    let cover = cover_texture(ui, st, shared, a);
    ui.horizontal(|ui| {
        if t::quiet(ui, "< Albums").clicked() {
            st.album = None;
        }
        if let Some(tex) = &cover {
            ui.image((tex.id(), egui::vec2(48.0, 48.0)));
        }
        t::heading(ui, &a.name);
        badge(ui, &a.scope_name, t::ACCENT);
        if a.archived {
            badge(ui, "archived", t::FAINT);
        }
    });
    if !a.description.is_empty() {
        t::muted(ui, &a.description);
    }
    if let Some(b) = &a.created_by {
        t::small(ui, &format!("made by {}", b.being));
    }
    ui.add_space(t::GAP);
    t::card(ui, |ui| {
        t::section_label(ui, "Songs, in order");
        if a.songs.is_empty() {
            t::muted(ui, "Empty. Add a song below, or from a song's Org & access.");
        }
        let n = a.songs.len();
        for (i, row) in a.songs.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{:>2}", i + 1)).font(t::code(t::S12)).color(t::FAINT));
                if row.locked {
                    ui.label(egui::RichText::new("1 private song").font(t::text(t::S12)).color(t::FAINT));
                } else if let Some(dir) = &row.dir {
                    if t::quiet(ui, row.name.as_deref().unwrap_or("song")).clicked() {
                        shared.act(Request::OpenSong { dir: dir.clone() });
                    }
                } else {
                    ui.label(egui::RichText::new(row.song.short()).font(t::code(t::S12)));
                }
                if let Some(o) = lib.songs.iter().find(|x| x.song == Some(row.song)).and_then(|x| x.org).and_then(|o| lib.orgs.iter().find(|r| r.org == o)) {
                    badge(ui, &o.name, t::ACCENT);
                }
                badge(ui, access_word(row.access), t::MUTED);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if t::quiet(ui, "Remove").clicked() {
                        shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::RemoveSong { song: row.song } });
                    }
                    if i + 1 < n && t::quiet(ui, "v").clicked() {
                        let mut order: Vec<_> = a.songs.iter().map(|x| x.song).collect();
                        order.swap(i, i + 1);
                        shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::Reorder(order) });
                    }
                    if i > 0 && t::quiet(ui, "^").clicked() {
                        let mut order: Vec<_> = a.songs.iter().map(|x| x.song).collect();
                        order.swap(i, i - 1);
                        shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::Reorder(order) });
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            t::muted(ui, "Add song");
            let in_album: Vec<SongId> = a.songs.iter().map(|x| x.song).collect();
            egui::ComboBox::from_id_salt(("album-add", a.album)).selected_text(egui::RichText::new("pick a song").font(t::text(t::S12))).show_ui(ui, |ui| {
                for song in lib.songs.iter().filter(|x| x.song.map(|id| !in_album.contains(&id)).unwrap_or(false)) {
                    if ui.selectable_label(false, &song.name).clicked() {
                        if let Some(id) = song.song {
                            shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::AddSong { song: id, after: None } });
                        }
                    }
                }
            });
            if let Some(open) = s.song.as_ref().filter(|x| !in_album.contains(&x.id)) {
                if t::button(ui, &format!("Add \"{}\"", open.name)).clicked() {
                    shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::AddSong { song: open.id, after: None } });
                }
            }
        });
    });
    if a.can_curate {
        ui.add_space(t::GAP);
        t::card(ui, |ui| {
            t::section_label(ui, "Curate");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.add_song).hint_text("new name").desired_width(200.0).font(t::text(t::S12)));
                if t::button(ui, "Rename").clicked() && !st.add_song.trim().is_empty() {
                    shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::Rename(st.add_song.trim().into()) });
                    st.add_song.clear();
                }
                if t::button(ui, if a.archived { "Unarchive" } else { "Archive" }).clicked() {
                    shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::Archive(!a.archived) });
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.cover_path).hint_text("C:\\path\\to\\cover.png").desired_width(260.0).font(t::code(t::S12)));
                if t::button(ui, "Set cover").clicked() && !st.cover_path.trim().is_empty() {
                    shared.act(Request::AlbumCover { scope: a.scope.clone(), album: a.album, path: st.cover_path.trim().into() });
                    st.cover_path.clear();
                }
                if a.cover.is_some() && t::quiet(ui, "Remove cover").clicked() {
                    shared.act(Request::AlbumAct { scope: a.scope.clone(), album: a.album, action: AlbumAction::SetCover(None) });
                }
            });
        });
    }
    ui.add_space(t::GAP);
    t::section_label(ui, "The album's moments");
    let rows = shared.album_timeline.lock().clone();
    for r in rows.iter().rev().take(30) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{:>3}", r.number)).font(t::code(t::S11)).color(t::FAINT));
            ui.label(egui::RichText::new(&r.summary).font(t::text(t::S12)));
            t::small(ui, &r.being.being.0);
            if !r.sealed {
                badge(ui, "unsealed", t::WARN);
            }
        });
    }
}

// ----- ORGS -----

fn orgs(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared, lib: &LibraryView) {
    if let Some(o) = st.org {
        if let Some(row) = lib.orgs.iter().find(|r| r.org == o) {
            org_page(ui, st, s, shared, lib, row);
            return;
        }
        st.org = None;
    }
    t::card(ui, |ui| {
        t::section_label(ui, "New org");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.new_org).hint_text("Org name").desired_width(220.0).font(t::text(t::S12)));
            if t::primary(ui, "New org").clicked() && !st.new_org.trim().is_empty() {
                shared.act(Request::NewOrg { name: st.new_org.trim().into() });
                st.new_org.clear();
            }
        });
        t::small(ui, "A standing group of Studios: its members are crew in every song it shares.");
    });
    ui.add_space(t::GAP);
    if lib.orgs.is_empty() {
        t::muted(ui, "You are in no org yet.");
    }
    for o in &lib.orgs {
        ui.horizontal(|ui| {
            t::dot(ui, o.color.map(t::color).unwrap_or(t::ACCENT));
            if t::quiet(ui, &o.name).clicked() {
                st.org = Some(o.org);
                st.org_tab = OrgTab::Songs;
                st.org_label = o.name.clone();
                shared.act(Request::OrgMembers { org: o.org });
            }
            badge(ui, if o.owner { "owner" } else { "member" }, t::MUTED);
            t::small(ui, &format!("{} members · {} songs · {} albums", o.members, o.songs.len(), o.albums));
            if o.archived {
                badge(ui, "archived", t::FAINT);
            }
        });
    }
}

fn org_page(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared, _lib: &LibraryView, o: &OrgRow) {
    ui.horizontal(|ui| {
        if t::quiet(ui, "< Orgs").clicked() {
            st.org = None;
        }
        t::dot(ui, o.color.map(t::color).unwrap_or(t::ACCENT));
        t::heading(ui, &o.name);
        badge(ui, if o.owner { "you own this org" } else { "you are a member" }, t::MUTED);
        if o.archived {
            badge(ui, "archived", t::FAINT);
        }
    });
    let cur = match st.org_tab {
        OrgTab::Songs => 0,
        OrgTab::Albums => 1,
        OrgTab::Members => 2,
        OrgTab::Settings => 3,
    };
    let idx = t::segmented(ui, cur, &["Songs", "Albums", "Members", "Settings"]);
    st.org_tab = [OrgTab::Songs, OrgTab::Albums, OrgTab::Members, OrgTab::Settings][idx];
    ui.add_space(t::GAP);
    match st.org_tab {
        OrgTab::Songs => {
            if o.songs.is_empty() {
                t::muted(ui, "No songs under this org yet. Make one from Songs > New song > Under.");
            }
            for row in &o.songs {
                ui.horizontal(|ui| {
                    if t::quiet(ui, &row.name).clicked() {
                        shared.act(Request::OpenSong { dir: row.dir.clone() });
                    }
                    if t::button(ui, &format!("Open in {}", daw_word(&s.host))).clicked() {
                        shared.act(Request::OpenInDaw { dir: row.dir.clone() });
                    }
                    badge(ui, access_word(row.access), t::MUTED);
                    ui.label(egui::RichText::new(row.dir.display().to_string()).font(t::code(t::S11)).color(t::FAINT));
                });
            }
            if t::button(ui, "New song here").clicked() {
                st.song_org = Some(o.org);
                st.song_access = if o.defaults.access == Access::Org { 0 } else { 1 };
                st.tab = Tab::Songs;
                st.org = None;
            }
        }
        OrgTab::Albums => {
            let albums = shared.albums.lock().clone();
            let mine: Vec<&AlbumRow> = albums.iter().filter(|a| a.scope == AlbumScope::Org(o.org)).collect();
            if mine.is_empty() {
                t::muted(ui, "No albums yet.");
            }
            for a in mine {
                ui.horizontal(|ui| {
                    if t::quiet(ui, &a.name).clicked() {
                        st.album = Some((a.scope.clone(), a.album));
                        st.tab = Tab::Albums;
                        st.org = None;
                        shared.act(Request::AlbumTimeline { scope: a.scope.clone(), album: a.album });
                    }
                    t::small(ui, &format!("{} songs", a.songs.len()));
                });
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.new_album).hint_text("Album name").desired_width(200.0).font(t::text(t::S12)));
                if t::button(ui, "New album").clicked() && !st.new_album.trim().is_empty() {
                    shared.act(Request::NewAlbum { scope: AlbumScope::Org(o.org), name: st.new_album.trim().into() });
                    st.new_album.clear();
                }
            });
        }
        OrgTab::Members => {
            let members = shared.org_members.lock().clone();
            for m in &members {
                ui.horizontal(|ui| {
                    t::dot(ui, if m.connected { t::OK } else { t::FAINT });
                    t::strong(ui, &m.label);
                    badge(ui, if m.invited { "invited" } else if m.owner { "owner" } else { "member" }, t::MUTED);
                    for (_, name, kind, color) in &m.beings {
                        t::avatar(ui, *color, name.chars().next().unwrap_or('?'), true, 16.0).on_hover_text(format!("{name} ({kind})"));
                    }
                    if o.owner && m.id != s.name {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if m.invited {
                                if t::quiet(ui, "Cancel").clicked() {
                                    shared.act(Request::OrgRemove { org: o.org, id: m.id });
                                }
                            } else {
                                if t::danger(ui, "Remove").clicked() {
                                    shared.act(Request::OrgRemove { org: o.org, id: m.id });
                                }
                                if m.owner {
                                    if t::quiet(ui, "Demote").clicked() {
                                        shared.act(Request::OrgDemote { org: o.org, id: m.id });
                                    }
                                } else if t::quiet(ui, "Promote").clicked() {
                                    shared.act(Request::OrgPromote { org: o.org, id: m.id });
                                }
                            }
                        });
                    }
                });
            }
            if o.owner {
                ui.add_space(t::GAP);
                t::card(ui, |ui| {
                    t::section_label(ui, "Invite a Studio");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut st.invite_id).hint_text("paste their Studio ID").desired_width(320.0).font(t::code(t::S12)));
                        if t::button(ui, "Paste").clicked() {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                if let Ok(text) = cb.get_text() {
                                    st.invite_id = text.trim().to_string();
                                }
                            }
                        }
                        if t::primary(ui, "Invite").clicked() && !st.invite_id.trim().is_empty() {
                            shared.act(Request::OrgInvite { org: o.org, friend: st.invite_id.trim().to_string() });
                            st.invite_id.clear();
                        }
                    });
                    t::small(ui, "They see the invite in their Library; membership starts when they accept.");
                });
            }
        }
        OrgTab::Settings => {
            t::card(ui, |ui| {
                t::section_label(ui, "Org");
                ui.horizontal(|ui| {
                    ui.add_enabled(o.owner, egui::TextEdit::singleline(&mut st.org_label).desired_width(220.0).font(t::text(t::S12)));
                    if o.owner && t::button(ui, "Rename").clicked() && !st.org_label.trim().is_empty() {
                        shared.act(Request::OrgSettings { org: o.org, label: Some(st.org_label.trim().into()), color: None, defaults: None, archived: None });
                    }
                });
                ui.horizontal(|ui| {
                    t::muted(ui, "Color");
                    for c in t::BEING_COLORS {
                        if t::avatar(ui, c, ' ', true, 14.0).clicked() && o.owner {
                            shared.act(Request::OrgSettings { org: o.org, label: None, color: Some(c), defaults: None, archived: None });
                        }
                    }
                });
                ui.horizontal(|ui| {
                    t::muted(ui, "Keeper");
                    match &o.keeper {
                        Some((n, m)) => t::value(ui, &format!("{} on {m}{}", n.short_code(), if o.i_am_keeper { " (this machine)" } else { "" }), 0.0),
                        None => t::value(ui, "none", 0.0),
                    }
                    if o.owner && !o.i_am_keeper && t::button(ui, "Move keeper here").clicked() {
                        shared.act(Request::OrgMoveKeeper { org: o.org, name: s.name, machine: String::new() });
                    }
                });
            });
            ui.add_space(t::GAP);
            t::card(ui, |ui| {
                t::section_label(ui, "Defaults for new songs");
                let d = &o.defaults;
                ui.horizontal(|ui| {
                    t::muted(ui, "Access");
                    let cur = if d.access == Access::Org { 0 } else { 1 };
                    let idx = t::segmented(ui, cur, &["Org", "Private"]);
                    if idx != cur && o.owner {
                        let mut nd = d.clone();
                        nd.access = if idx == 0 { Access::Org } else { Access::Private };
                        shared.act(Request::OrgSettings { org: o.org, label: None, color: None, defaults: Some(nd), archived: None });
                    }
                });
                ui.horizontal(|ui| {
                    t::muted(ui, "ai members");
                    let cur = if d.ai_policy == flit_core::AiPolicy::Grants { 0 } else { 1 };
                    let idx = t::segmented(ui, cur, &["grants work", "own history only"]);
                    if idx != cur && o.owner {
                        let mut nd = d.clone();
                        nd.ai_policy = if idx == 0 { flit_core::AiPolicy::Grants } else { flit_core::AiPolicy::OwnHistoryOnly };
                        shared.act(Request::OrgSettings { org: o.org, label: None, color: None, defaults: Some(nd), archived: None });
                    }
                });
                ui.horizontal(|ui| {
                    let mut show = d.show_locked;
                    if ui.checkbox(&mut show, "Show private songs in org albums as a locked row").changed() && o.owner {
                        let mut nd = d.clone();
                        nd.show_locked = show;
                        shared.act(Request::OrgSettings { org: o.org, label: None, color: None, defaults: Some(nd), archived: None });
                    }
                });
            });
            ui.add_space(t::GAP);
            ui.horizontal(|ui| {
                if t::danger(ui, "Leave org").clicked() {
                    shared.act(Request::OrgLeave { org: o.org });
                    st.org = None;
                }
                if o.owner && t::quiet(ui, if o.archived { "Unarchive org" } else { "Archive org" }).clicked() {
                    shared.act(Request::OrgSettings { org: o.org, label: None, color: None, defaults: None, archived: Some(!o.archived) });
                }
            });
        }
    }
}

// ----- INVITES -----

fn invites(ui: &mut egui::Ui, shared: &Shared, lib: &LibraryView) {
    if lib.invites.is_empty() {
        t::muted(ui, "No invites waiting.");
        return;
    }
    for i in &lib.invites {
        t::card(ui, |ui| {
            ui.horizontal(|ui| {
                t::strong(ui, &i.name);
                badge(ui, if i.kind == ContainerKind::Org { "org" } else { "song" }, t::ACCENT);
                t::muted(ui, &format!("from {}", i.from_label));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if t::quiet(ui, "Decline").clicked() {
                        shared.act(Request::DeclineInvite { container: i.container });
                    }
                    if i.kind == ContainerKind::Org {
                        if t::primary(ui, "Accept").clicked() {
                            shared.act(Request::AcceptOrgInvite { org: i.container });
                        }
                    } else {
                        t::small(ui, "song invites: Home > Join");
                    }
                });
            });
        });
    }
}
