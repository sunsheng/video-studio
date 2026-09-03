# LTX 2.5

The current validated local combination is:

- diffusion: `ltx-2.5-22b-distilled-transformer-comfy-int8-convrot.safetensors`
- prompt/text encoder: `gemma4_e2b_it_int8_convrot.safetensors`
- conditioning encoder: `gemma4-12b-with-proj-ltx-2.5-comfy-int8-convrot.safetensors`
- video VAE: `ltx-2.5-video-vae-bf16.safetensors`
- audio VAE: `ltx-2.5-audio-vae-bf16.safetensors`
- latent upscaler: `ltx-2.5-latent-spatial-upscaler-x2-bf16-1.0.safetensors`

Validated native modes:

- `t2v_api.json`: text-to-video, prompt_id `5e61ef26-89e8-400f-bd3d-b11f87c72a1f`.
- `i2v_api.json`: image-to-video, prompt_id `1ffd2799-e4cb-426b-82e8-c90d0432f376`.
- `flf2v_api.json`: first/last-frame interpolation, prompt_id `b873f754-292a-406b-9db8-652c10368c75`.

Keep the two Gemma roles distinct; they are not interchangeable model variants.
Only prompt, seed, dimensions/frame count, declared image inputs, and output
prefix should be overridden. These short executions are `render_smoke`
evidence, not full-flow production acceptance.
