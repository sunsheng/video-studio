# MiniMax H3

Fixed validated combinations on `http://127.0.0.1:9001`:

- T2V/I2V: `minimax_h3_fl2va_int8_convrot.safetensors` +
  `qwen3vl_32b_minimax_h3_int8_convrot.safetensors` +
  `minimax_h3_video_vae_fp16.safetensors` +
  `minimax_h3_audio_vae_fp32.safetensors`.
- R2V: replace only the diffusion model with
  `minimax_h3_ref2va_int8_convrot.safetensors`.

Validated modes:

- `t2v_api.json`: T2V run `20260830T204108Z-622bb0`, prompt_id `1853143f-4bf7-4d71-afd8-247473e9b7ef`.
- `i2v_api.json`: first/last-frame I2V run `20260831T035154Z-29b70a`, prompt_id `c667ea2b-5bf1-4436-80a2-bffd456cf226`.
- `ref2v_api.json`: image-only R2V run `20260830T205304Z-0e6d02`; the unified compiler added image + video + same-video audio + independent audio references in run `20260831T035154Z-29b70a`, prompt_id `0dd46fbe-c90c-49e1-add0-b2ce6f3d59ca`.

I2V accepts a required first frame and an optional last frame. R2V accepts
image, video, and audio references through the native node. Only prompt, seed,
dimensions/frame count, declared references, and output prefix may be changed.
