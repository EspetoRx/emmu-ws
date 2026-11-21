//! Core WonderSwan / WonderSwan Color do Emmu.
//!
//! O núcleo de emulação é o `swanium-core` (MIT), vendorizado em
//! `vendor/swanian-core`. É Rust puro, headless, e compila para WebAssembly via
//! `wasm-pack`. A camada deste crate expõe a mesma API dos demais cores do
//! worker de WebAssembly:
//!
//! - `new(rom)` / `gameTitle()`
//! - `runFrame(renderGraphics)` → RGBA8888 (224x144)
//! - `audioSamples()` / `audioSampleRate()` / `audioChannels()`
//! - `setButton(button, pressed, player)` / `setVolume(volume)`
//! - `saveState()` / `loadState(bytes)` / `restart()`
//!
//! No desktop o WonderSwan roda no Mesen via `emmu-mesen-sys`; este crate é
//! usado no navegador e também pode ser usado como biblioteca Rust nativa.

use swanium_core::keypad::KeyState;
use swanium_core::system::{CYCLES_PER_FRAME, MASTER_CLOCK_HZ, System};

/// Largura da tela LCD do WonderSwan.
pub const FRAME_WIDTH: usize = 224;
/// Altura da tela LCD do WonderSwan.
pub const FRAME_HEIGHT: usize = 144;
/// Taxa de saída do APU do Swanium (estéreo intercalado).
pub const AUDIO_SAMPLE_RATE: u32 = 24_000;
/// O APU do Swanium entrega amostras interleaved L/R.
pub const AUDIO_CHANNELS: u32 = 2;
/// O worker da UI chama `runFrame` a 60 Hz, mas o WonderSwan roda a ~75,47 Hz.
/// Acumulamos `MASTER_CLOCK_HZ / 60` ciclos por chamada para manter áudio e
/// jogo em tempo real (e não em câmera lenta) sem mudar a taxa do APU.
const CYCLES_PER_UI_FRAME: u64 = MASTER_CLOCK_HZ as u64 / 60;

/// Prefixo dos save states do Emmu, para rejeitar dados de outros cores.
const STATE_MAGIC: &[u8; 8] = b"EMMUWS01";
const STATE_HEADER_LEN: usize = STATE_MAGIC.len() + std::mem::size_of::<u32>();
const STATE_VERSION: u32 = 1;

fn button_mask(button: &str, player: u8) -> Option<u16> {
    if player != 1 {
        return None;
    }
    // O WonderSwan tem dois direcionais de 4 vias (X e Y), A, B e Start.
    // O botão de som do hardware não é exposto pelo core Swanium.
    let key = match button {
        "Up" => KeyState::X1,
        "Right" => KeyState::X2,
        "Down" => KeyState::X3,
        "Left" => KeyState::X4,
        // O segundo direcional usa os nomes U/D/L/R da ABI do Mesen.
        "U" => KeyState::Y1,
        "R" => KeyState::Y2,
        "D" => KeyState::Y3,
        "L" => KeyState::Y4,
        "A" => KeyState::A,
        "B" => KeyState::B,
        "Start" => KeyState::START,
        // Sound/Select/X/Y não existem no hardware emulado.
        _ => return None,
    };
    Some(key.bits())
}

/// Fachada nativa do core WonderSwan.
pub struct WsCore {
    system: System,
    rom: Vec<u8>,
    keys: KeyState,
    frame: Vec<u8>,
    audio: Vec<f32>,
    audio_rate: u32,
    audio_channels: u32,
    volume: f32,
    cycle_carry: u64,
}

impl WsCore {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        if rom.is_empty() {
            return Err("ROM vazia.".into());
        }
        // WonderSwan exige no mínimo o footer de 16 bytes no final da ROM.
        if rom.len() < 16 {
            return Err("ROM WonderSwan muito pequena.".into());
        }
        let system = System::from_rom(rom.to_vec());
        Ok(Self {
            system,
            rom: rom.to_vec(),
            keys: KeyState::NONE,
            frame: vec![0; FRAME_WIDTH * FRAME_HEIGHT * 4],
            audio: Vec::new(),
            audio_rate: AUDIO_SAMPLE_RATE,
            audio_channels: AUDIO_CHANNELS,
            volume: 1.0,
            cycle_carry: 0,
        })
    }

    pub fn game_title(&self) -> &str {
        // O footer do WonderSwan não contém um título textual; a UI usa o nome
        // do arquivo. Este valor é apenas o rótulo genérico do hardware.
        "WonderSwan"
    }

    /// Avança o tempo de um frame da UI (1/60 s) e devolve o último frame
    /// RGBA8888 (224x144) quando `render` é verdadeiro. O WonderSwan roda mais
    /// rápido que 60 Hz, então às vezes são necessários dois frames internos
    /// por chamada para não atrasar jogo e áudio.
    pub fn run_frame(&mut self, render: bool) -> Result<Vec<u8>, String> {
        self.cycle_carry += CYCLES_PER_UI_FRAME;
        while self.cycle_carry >= CYCLES_PER_FRAME as u64 {
            self.system.run_frame(self.keys);
            self.cycle_carry -= CYCLES_PER_FRAME as u64;
        }
        if render {
            for (index, &color) in self.system.framebuffer().iter().enumerate() {
                let r = ((color >> 8) & 0x0F) as u8 * 17;
                let g = ((color >> 4) & 0x0F) as u8 * 17;
                let b = (color & 0x0F) as u8 * 17;
                let base = index * 4;
                self.frame[base] = r;
                self.frame[base + 1] = g;
                self.frame[base + 2] = b;
                self.frame[base + 3] = 0xFF;
            }
            Ok(self.frame.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// Converte e drena as amostras i16 intercaladas do APU para f32.
    pub fn take_audio_samples(&mut self) -> Vec<f32> {
        let samples = self.system.audio_samples();
        let mut output = Vec::with_capacity(samples.len());
        for &sample in samples {
            output.push((sample as f32 / 32768.0) * self.volume);
        }
        self.system.clear_audio_samples();
        output
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.audio_rate
    }

    pub fn audio_channels(&self) -> u32 {
        self.audio_channels
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.volume = (volume.clamp(0.0, 1.0)) as f32;
    }

    pub fn set_button(&mut self, button: &str, pressed: bool, player: u8) -> Result<(), String> {
        let Some(mask) = button_mask(button, player) else {
            return Ok(());
        };
        let bits = if pressed {
            self.keys.bits() | mask
        } else {
            self.keys.bits() & !mask
        };
        self.keys = KeyState::from_bits(bits);
        Ok(())
    }

    pub fn restart(&mut self) {
        self.system = System::from_rom(self.rom.clone());
        self.keys = KeyState::NONE;
        self.audio.clear();
        self.cycle_carry = 0;
    }

    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let inner = self
            .system
            .save_state_bytes()
            .map_err(|error| format!("Falha ao salvar estado WS: {error}"))?;
        let mut output = Vec::with_capacity(STATE_HEADER_LEN + inner.len());
        output.extend_from_slice(STATE_MAGIC);
        output.extend_from_slice(&STATE_VERSION.to_le_bytes());
        output.extend_from_slice(&inner);
        Ok(output)
    }

    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() < STATE_HEADER_LEN || &bytes[..STATE_MAGIC.len()] != STATE_MAGIC {
            return Err("Estado salvo inválido para WonderSwan.".into());
        }
        let version = u32::from_le_bytes([
            bytes[STATE_MAGIC.len()],
            bytes[STATE_MAGIC.len() + 1],
            bytes[STATE_MAGIC.len() + 2],
            bytes[STATE_MAGIC.len() + 3],
        ]);
        if version != STATE_VERSION {
            return Err(format!("Versão de save state WS não suportada: {version}"));
        }
        self.system
            .load_state_bytes(&bytes[STATE_HEADER_LEN..])
            .map_err(|error| format!("Falha ao carregar estado WS: {error}"))
    }

    /// Bytes persistentes do cartucho (SRAM/EEPROM), quando houver.
    pub fn save_ram(&self) -> Vec<u8> {
        self.system.save_data().to_vec()
    }

    pub fn load_save_ram(&mut self, data: &[u8]) {
        self.system.load_save_data(data);
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct WsEmulator {
        core: WsCore,
    }

    #[wasm_bindgen]
    impl WsEmulator {
        #[wasm_bindgen(constructor)]
        pub fn new(rom: &[u8]) -> Result<WsEmulator, JsValue> {
            Ok(Self {
                core: WsCore::new(rom).map_err(|error| JsValue::from_str(&error))?,
            })
        }

        #[wasm_bindgen(js_name = gameTitle)]
        pub fn game_title(&self) -> String {
            self.core.game_title().to_owned()
        }

        #[wasm_bindgen(js_name = runFrame)]
        pub fn run_frame(&mut self, render_graphics: bool) -> Result<Vec<u8>, JsValue> {
            self.core
                .run_frame(render_graphics)
                .map_err(|error| JsValue::from_str(&error))
        }

        #[wasm_bindgen(js_name = audioSamples)]
        pub fn audio_samples(&mut self) -> Vec<f32> {
            self.core.take_audio_samples()
        }

        #[wasm_bindgen(js_name = audioSampleRate)]
        pub fn audio_sample_rate(&self) -> u32 {
            self.core.audio_sample_rate()
        }

        #[wasm_bindgen(js_name = audioChannels)]
        pub fn audio_channels(&self) -> u32 {
            self.core.audio_channels()
        }

        #[wasm_bindgen(js_name = setButton)]
        pub fn set_button(
            &mut self,
            button: &str,
            pressed: bool,
            player: u32,
        ) -> Result<(), JsValue> {
            self.core
                .set_button(button, pressed, player as u8)
                .map_err(|error| JsValue::from_str(&error))
        }

        #[wasm_bindgen(js_name = setVolume)]
        pub fn set_volume(&mut self, volume: f64) {
            self.core.set_volume(volume);
        }

        pub fn restart(&mut self) {
            self.core.restart();
        }

        #[wasm_bindgen(js_name = saveState)]
        pub fn save_state(&mut self) -> Result<Vec<u8>, JsValue> {
            self.core
                .save_state()
                .map_err(|error| JsValue::from_str(&error))
        }

        #[wasm_bindgen(js_name = loadState)]
        pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
            self.core
                .load_state(bytes)
                .map_err(|error| JsValue::from_str(&error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ROM mínima: HLT no vetor de reset (os últimos 16 bytes do arquivo).
    fn minimal_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 64 * 1024];
        let last = rom.len() - 16;
        rom[last] = 0xF4;
        rom
    }

    #[test]
    fn runs_frame_and_returns_rgba() {
        let mut core = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        let frame = core.run_frame(true).expect("deve avançar um frame");
        assert_eq!(frame.len(), FRAME_WIDTH * FRAME_HEIGHT * 4);
        assert_eq!(frame[3], 0xFF);
    }

    #[test]
    fn accepts_both_dpads_and_action_buttons() {
        let mut core = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        for button in [
            "Up", "Down", "Left", "Right", "U", "D", "L", "R", "A", "B", "Start", "Select",
        ] {
            core.set_button(button, true, 1).expect("botão válido");
            core.set_button(button, false, 1).expect("botão válido");
        }
        assert_eq!(core.keys.bits(), 0);
    }

    #[test]
    fn audio_is_stereo_at_apu_rate() {
        let core = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        assert_eq!(core.audio_sample_rate(), 24_000);
        assert_eq!(core.audio_channels(), 2);
    }

    #[test]
    fn volume_scales_samples() {
        let mut quiet = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        quiet.set_volume(0.0);
        quiet.run_frame(true).expect("deve avançar um frame");
        assert!(
            quiet
                .take_audio_samples()
                .iter()
                .all(|&sample| sample == 0.0)
        );
    }

    #[test]
    fn save_state_round_trip() {
        let mut core = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        core.run_frame(true).expect("deve avançar um frame");
        let state = core.save_state().expect("deve salvar estado");
        assert_eq!(&state[..8], b"EMMUWS01");
        core.run_frame(true).expect("deve avançar um frame");
        core.load_state(&state).expect("deve carregar estado");
    }

    #[test]
    fn rejects_foreign_save_state() {
        let mut core = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        assert!(core.load_state(b"EMMUGG01\0\0\0\0").is_err());
    }

    #[test]
    fn restart_returns_to_power_on_state() {
        let mut core = WsCore::new(&minimal_rom()).expect("ROM mínima deve carregar");
        core.run_frame(true).expect("deve avançar um frame");
        core.set_button("A", true, 1).expect("botão válido");
        core.restart();
        assert_eq!(core.keys.bits(), 0);
        core.run_frame(true).expect("deve avançar um frame");
    }

    #[test]
    fn real_rom_smoke_test() {
        let Ok(path) = std::env::var("EMMU_WS_TEST_ROM") else {
            return;
        };
        let rom = std::fs::read(&path).expect("não foi possível ler EMMU_WS_TEST_ROM");
        let mut core = WsCore::new(&rom).expect("ROM real deve carregar");
        for _ in 0..120 {
            let frame = core.run_frame(true).expect("deve avançar um frame");
            assert_eq!(frame.len(), FRAME_WIDTH * FRAME_HEIGHT * 4);
            let _ = core.take_audio_samples();
        }
    }
}
