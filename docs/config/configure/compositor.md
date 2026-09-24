# gnoblin.configure.compositor

Configure this part of `gnoblin.configure` with the `compositor` key.

Inside `gnoblin.configure {compositor = {...}}`. Values apply on reload.
These settings cover compositor interaction preferences; accessibility
settings remain separate.

| Key                           | Values                                | Default              |
| ----------------------------- | ------------------------------------- | -------------------- |
| `enable_animations`           | Boolean                               | `true`               |
| `locate_pointer`              | Boolean                               | `false`              |
| `visual_bell`, `audible_bell` | Boolean                               | `false`, `true`      |
| `visual_bell_type`            | `"fullscreen-flash"`, `"frame-flash"` | `"fullscreen-flash"` |

Guide: [session settings](/guides/session_settings).
