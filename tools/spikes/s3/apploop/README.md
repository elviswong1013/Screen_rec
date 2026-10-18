# apploop（S3 spike，非生产代码）

WASAPI 按进程（进程树）application loopback 录制 + FFT 检查器。
背景与实测结论：`docs/spikes/s3/s3-audio-isolation.md`；运行入口：`tools/spikes/s3/run_s3_tests.ps1`。

依赖：`wasapi` 0.26.0（内置 `ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_PARAMS` 流程）。

```
apploop record  --pid <PID> --out <file.wav> [--seconds 8] [--exclude-tree]
                [--pad-silence] [--no-autoconvert] [--sta] [--pre-sta] [--device-loopback]
                [--channels 2] [--rate 48000] [--log <file>]
                （可重复 --pid/--out 对，在同一进程内并发多客户端；--device-loopback 为整机混音对照）
apploop analyze --input <file.wav> [--freqs 440,880] [--ref 440]
                [--nfft 32768] [--threshold-db -40] [--csv <file.csv>] [--block-s 0.5]
apploop probe
```

要点：

- application loopback 无物理端点，`GetMixFormat` 不可用；采样格式由调用方指定（本 spike 48 kHz f32）。
- COM 必须在采集线程初始化（默认 MTA；`--sta`/`--pre-sta` 用于演示 UI 线程场景的失败模式）。
- `AudioClient`/`AudioCaptureClient` 是 `!Send`：客户端生命周期保持在单一线程内（多客户端=每目标一线程）。
- 本 crate 在 `Cargo.toml` 中声明了自有 `[workspace]`：独立于仓库根 workspace 构建（根成员未就绪时也可用）。
