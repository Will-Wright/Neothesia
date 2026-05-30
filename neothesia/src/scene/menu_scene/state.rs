use std::collections::VecDeque;

use crate::{NeothesiaEvent, context::Context, output_manager::OutputDescriptor, song::Song};

type InputDescriptor = midi_io::MidiInputPort;

pub struct UiState {
    pub outputs: Vec<OutputDescriptor>,
    pub selected_output: Option<OutputDescriptor>,

    pub inputs: Vec<InputDescriptor>,
    pub selected_input: Option<InputDescriptor>,

    pub is_loading: bool,

    pub song: Option<Song>,

    page_stack: VecDeque<Page>,
}

impl UiState {
    pub fn new(_ctx: &Context, song: Option<Song>) -> Self {
        let mut page_stack = VecDeque::new();
        page_stack.push_front(Page::Main);

        Self {
            outputs: Vec::new(),
            selected_output: None,
            inputs: Vec::new(),
            selected_input: None,
            is_loading: false,
            song,

            page_stack,
        }
    }

    pub fn song(&self) -> Option<&Song> {
        self.song.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn current(&self) -> &Page {
        self.page_stack.front().unwrap()
    }

    pub fn go_to(&mut self, page: Page) {
        self.page_stack.push_front(page);
    }

    pub fn go_back(&mut self) {
        match self.page_stack.len() {
            1 => {
                // Last page in the stack, let's go to exit page
                self.page_stack.push_front(Page::Exit);
            }
            _ => {
                self.page_stack.pop_front();
            }
        }
    }
}

impl UiState {
    pub fn tick(&mut self, ctx: &mut Context) {
        self.outputs = ctx.output_manager.outputs();
        self.inputs = ctx.input_manager.inputs();

        if self.selected_output.is_none() {
            // NEOTHESIA_OUTPUT=dummy/no-output/off forces silent primary output.
            // NEOTHESIA_OUTPUT=<substr> forces routing to a MIDI-out port whose
            // display name contains the substring (e.g. "NeothesiaOut" matches
            // "IAC Driver NeothesiaOut"). This keeps practice workflows
            // launchable without relying on UI settings persistence.
            //
            // Why this hook lives in `UiState::tick()` and not in
            // `OutputManager::new()`: a copy placed in OutputManager::new()
            // gets clobbered ~50ms later when this tick() runs and walks the
            // `selected_output.is_none()` → `config.output()` fallback chain.
            // `default_output()` returns `Some("Buildin Synth")`, so the menu
            // resolves selected_output = Synth, then `NEOTHESIA_AUTOPLAY` →
            // `play()` → `connect_io()` → `output_manager.connect(Synth)`
            // overrides any prior MIDI-out pick. Setting `selected_output`
            // here, *before* the config-based fallback, is what makes the
            // env var actually win at play time.
            if let Some(filter) = std::env::var_os("NEOTHESIA_OUTPUT") {
                let filter = filter.to_string_lossy().into_owned();
                if env_output_requests_dummy(&filter) {
                    log::info!("NEOTHESIA_OUTPUT={filter:?}: selecting dummy output");
                    self.selected_output = Some(OutputDescriptor::DummyOutput);
                } else {
                    let matched = self.outputs.iter().find(|o| match o {
                        OutputDescriptor::MidiOut(info) => info.to_string().contains(&filter),
                        _ => false,
                    });
                    if let Some(out) = matched {
                        log::info!("NEOTHESIA_OUTPUT: selecting MIDI output matching {filter:?}");
                        self.selected_output = Some(out.clone());
                    } else {
                        log::warn!(
                            "NEOTHESIA_OUTPUT={filter:?} set but no matching MIDI output found; using default"
                        );
                    }
                }
            }

            if self.selected_output.is_none() {
                if let Some(name) = ctx.config.output() {
                    if let Some(out) = self
                        .outputs
                        .iter()
                        .find(|output| output.to_string().as_str() == name)
                    {
                        self.selected_output = Some(out.clone());
                    } else {
                        self.selected_output = self.outputs.first().cloned();
                    }
                } else {
                    self.selected_output = Some(OutputDescriptor::DummyOutput);
                }
            }
        }

        if self.selected_input.is_none() {
            if let Some(input) = self
                .inputs
                .iter()
                .find(|input| Some(input.to_string().as_str()) == ctx.config.input())
            {
                self.selected_input = Some(input.clone());
            } else {
                self.selected_input = self.inputs.first().cloned();
            }
        }
    }
}

fn env_output_requests_dummy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "dummy" | "none" | "off" | "no output" | "no-output" | "silent"
    )
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Page {
    Exit,
    Main,
    Settings,
    TrackSelection,
}

fn connect_io(data: &UiState, ctx: &mut Context) {
    if let Some(out) = data.selected_output.clone() {
        let out = match out {
            #[cfg(feature = "synth")]
            OutputDescriptor::Synth(_) => {
                OutputDescriptor::Synth(ctx.config.soundfont_path().cloned())
            }
            o => o,
        };

        ctx.output_manager.connect(out);
        ctx.output_manager
            .connection()
            .set_gain(ctx.config.audio_gain());
    }

    if let Some(port) = data.selected_input.clone() {
        ctx.input_manager.connect_input(port);
    }
}

pub fn play(data: &UiState, ctx: &mut Context) {
    let Some(song) = data.song.as_ref() else {
        return;
    };

    connect_io(data, ctx);

    ctx.proxy
        .send_event(NeothesiaEvent::Play(song.clone()))
        .ok();
}

pub fn freeplay(data: &UiState, ctx: &mut Context) {
    connect_io(data, ctx);

    ctx.proxy
        .send_event(NeothesiaEvent::FreePlay(data.song.clone()))
        .ok();
}

#[cfg(test)]
mod tests {
    use super::env_output_requests_dummy;

    #[test]
    fn neothesia_output_dummy_aliases_request_silent_primary_output() {
        for value in ["dummy", "none", "off", "no output", "no-output", "silent"] {
            assert!(env_output_requests_dummy(value), "{value}");
            assert!(env_output_requests_dummy(&value.to_ascii_uppercase()), "{value}");
        }
        assert!(!env_output_requests_dummy("NeothesiaOut"));
    }
}
