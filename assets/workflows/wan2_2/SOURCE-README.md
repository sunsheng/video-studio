# Wan2.2 14B

Fixed non-Turbo baselines validated on `http://127.0.0.1:9001`:

- T2V: `wan2.2_t2v_high_noise_14B_fp8_scaled.safetensors` +
  `wan2.2_t2v_low_noise_14B_fp8_scaled.safetensors`.
- I2V/FLF2V: `wan2.2_i2v_high_noise_14B_fp8_scaled.safetensors` +
  `wan2.2_i2v_low_noise_14B_fp8_scaled.safetensors`.
- Shared: `umt5_xxl_fp8_e4m3fn_scaled.safetensors` +
  `wan_2.1_vae.safetensors`.

Validated native modes:

- `t2v_api.json`: prompt_id `d2ff2105-69b2-4a3f-9924-918343eddbe1`.
- `i2v_api.json`: prompt_id `58e715a3-22dc-428e-a5c5-70cb17c1d7c2`.
- `flf2v_api.json`: prompt_id `d4733c66-611e-4e41-bb8f-70023735d743`.

The API graphs are project-local adaptations of the three official ComfyUI
templates. They select the non-Turbo 20-step high/low-noise branches and do not
load a LoRA. Only prompt, seed, dimensions/duration or frame count, declared
image inputs, and output prefix may be overridden. These short executions are
`render_smoke` evidence, not full-flow production acceptance. WAN Animate 2 is
a different historical family and is not a substitute for these workflows.
