# emmu-ws

Core WonderSwan / WonderSwan Color do Emmu para a versão web. No desktop o
WonderSwan continua emulado pelo Mesen (`emmu-mesen-sys`).

## Dependências

Todo o código de emulação está dentro desta pasta, em `vendor/swanian-core`.
O upstream é [Swanium](https://github.com/bubio/Swanium), licenciado sob MIT.
Nenhum crate de emulação é buscado de fora do diretório do módulo.

## API

A fachada Rust `WsCore` e a classe WASM `WsEmulator` expõem:

| Método | Descrição |
| --- | --- |
| `new(rom)` | cria o core a partir dos bytes da ROM |
| `gameTitle()` | rótulo genérico (`WonderSwan`) |
| `runFrame(renderGraphics)` | avança um frame e devolve RGBA8888 224x144 |
| `audioSamples()` | amostras f32 estéreo intercaladas (L, R, L, R...) |
| `audioSampleRate()` | `24000` |
| `audioChannels()` | `2` |
| `setButton(button, pressed, player)` | controla os botões |
| `setVolume(volume)` | volume 0.0–1.0 aplicado às amostras |
| `restart()` | volta ao estado de power-on |
| `saveState()` / `loadState(bytes)` | save state com magic `EMMUWS01` |

## Controles

O hardware tem dois direcionais de 4 vias (X e Y), A, B e Start.

| Ação do player | Botão WS |
| --- | --- |
| Up / Down / Left / Right | direcional X |
| U / D / L / R | direcional Y |
| A / B / Start | A / B / Start |
| Select | sem efeito (o core não expõe o botão Sound) |

O segundo direcional usa os mesmos nomes da ABI do Mesen (`U`, `D`, `L`, `R`),
de modo que o mesmo evento do frontend funciona no web e no desktop.

## Save state

O formato é `EMMUWS01` + versão u32 + estado bincode do `swanian-core`. Estados de outros
cores são rejeitados. O histórico e a recuperação ficam no
`libraryStore.js`, como nos demais consoles.

## Build WASM

Gerado pelo script `build:wasm` da UI web:

```sh
npm run build:wasm
# saída: platforms/web/ui/src/wasm/ws/emmu_ws.js
```

Não edite `src/wasm/` diretamente.

## Testes

```sh
cargo test -p emmu-ws
EMMU_WS_TEST_ROM="/caminho/jogo.wsc" cargo test -p emmu-ws
```

O segundo comando é opcional e roda um smoke test com ROM real quando a
variável de ambiente está definida.
