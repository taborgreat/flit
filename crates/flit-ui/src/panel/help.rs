//! Help: every concept explained by what it is for, in plain words.

use crate::theme as t;
use flit_ipc::Status;

struct Topic {
    title: &'static str,
    purpose: &'static str,
    how: &'static str,
}

const TOPICS: &[Topic] = &[
    Topic {
        title: "Moment",
        purpose: "So nothing you do is ever lost. Every knob turn, fader move, plugin added or removed, note, take, save and message is one moment.",
        how: "A moment is made by a member and sealed by the song's keeper. The Timeline bar at the bottom shows every moment; click one to see the song as it was then (your DAW shows it as a read-only look), right-click to go back there or start a new history from it.",
    },
    Topic {
        title: "History",
        purpose: "So you can try something without touching what everyone else hears. A song starts with one history, main. Start a new one from any moment, even one before someone else's change.",
        how: "Everyone in a history is live with everyone else in it. Switch with the History dropdown in the Timeline bar. Bring a history (or chosen tracks of it) into another with Merge.",
    },
    Topic {
        title: "Track",
        purpose: "A mixer, instrument, bus or master track in your DAW. Tracks have no owners: anyone in the crew can work on any track, live. Coordination is social, through presence, not locks.",
        how: "Tracks appear as your DAW reports them. Their names come from your DAW; the Home screen lists them with who is looking at each one.",
    },
    Topic {
        title: "Member",
        purpose: "So the song knows who did what, including the AI and scripted helpers you run. A member is who acts: human, AI or scripted. You act as one member at a time.",
        how: "Add members on the Studio screen and switch with \"acting as\" in the top bar. Each member has its own color on the tape. AI and scripted members land in their own history unless you grant them tracks.",
    },
    Topic {
        title: "Studio",
        purpose: "Your identity: one key that holds authority (crew decisions, grants, adding members, recovery). Members live under it.",
        how: "The Studio screen manages Studios on this machine: switch, create, back up, import, log out. Back it up: losing the key with no backup loses that Studio's authority (the songs stay on every crew machine).",
    },
    Topic {
        title: "Keeper",
        purpose: "So every copy of the song agrees on the order of moments without a server. One machine per song seals moments; by default the song owner's.",
        how: "Nobody waits for the keeper: your changes apply at once everywhere and are sealed when the keeper sees them. If the keeper is offline, moments wait (the status strip says how many). An owner can move the role to any machine in Settings.",
    },
    Topic {
        title: "Collisions",
        purpose: "So two people changing the same knob at the same time never silently overwrite each other.",
        how: "Different properties never conflict. If two members change the same property before seeing each other's change, the first one sealed wins and the other is saved as a history named after the member and the property. You get a notice with Merge.",
    },
    Topic {
        title: "Merge",
        purpose: "To bring work from one history into another: a whole history or chosen tracks.",
        how: "Press Merge in the Timeline bar. Conflicts are listed per property with both sides; pick a side for each. It lands as one moment, undoable as one.",
    },
    Topic {
        title: "Timeline",
        purpose: "To see and scrub through everything that happened, through any lens.",
        how: "Lenses: Song (every moment in seal order), a track and its plugins, song settings, Me (my moments in the order I made them), Team, any member. Axis: Moments puts one mark per moment; Time places them on a clock (display only; order never comes from time). Drag or scroll to go back, Ctrl+wheel to zoom, Live to snap back.",
    },
    Topic {
        title: "Presence",
        purpose: "So you can see where everyone is working and avoid stepping on each other.",
        how: "Where you are in your DAW is recorded as moments (hidden from the tape unless you turn on Show presence). Avatars on tracks and in the status strip show who is where; click one to follow them into your DAW. Tints inside the DAW are optional (Settings).",
    },
    Topic {
        title: "Save version",
        purpose: "A checkpoint: the project file of your DAW plus renders of the tracks that changed, so anyone (on any DAW) can hear and restore exactly this.",
        how: "Press Save version on the Home screen. With a DAW link the DAW saves itself; the version lands when the file is written.",
    },
    Topic {
        title: "Crew",
        purpose: "The Studios allowed in the song. Owners approve, invite, remove, block members and decide on flags.",
        how: "Share the Song code (Home); the owner approves the request in Crew. Or add a friend by their friend code and invite them.",
    },
    Topic {
        title: "Versions of FLit",
        purpose: "So everyone in a song runs the same FLit. Every moment records the version that made it.",
        how: "A song requires an exact version (set by an owner in Settings). Moments made by another version wait until that member updates. Settings > Updates checks for releases.",
    },
];

pub fn show(ui: &mut egui::Ui, s: &Status) {
    t::heading(ui, "Help");
    t::small(ui, "Every concept, by what it is for.");
    let _ = s;
    for topic in TOPICS {
        t::card(ui, |ui| {
            ui.label(egui::RichText::new(topic.title).font(t::text_medium(t::S14)));
            ui.label(egui::RichText::new(topic.purpose).font(t::text(t::S12)));
            ui.label(egui::RichText::new(topic.how).font(t::text(t::S12)).color(t::MUTED));
        });
    }
    t::card(ui, |ui| {
        ui.label(egui::RichText::new("Modes of this plugin").font(t::text_medium(t::S14)));
        ui.label(egui::RichText::new("Panel on the master track (this). Track (send) last on a track you want others to hear. Track (receive) to hear a friend's track. Talk to play live outside the song.").font(t::text(t::S12)).color(t::MUTED));
    });
}
