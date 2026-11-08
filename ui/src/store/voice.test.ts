import { useApp } from "./app";

const ready = { phase: "ready" as const, hands_free: false, stt_ready: true, tts_ready: true, problem: null };

describe("voice store", () => {
  beforeEach(() => useApp.setState({ voice: null, micLevel: 0, speakerLevel: null, downloads: {}, character: "idle" }));

  it("stores voice status from voice://state", () => {
    useApp.getState().setVoice(ready);
    expect(useApp.getState().voice).toEqual(ready);
    useApp.getState().setVoice({ ...ready, phase: "listening" });
    expect(useApp.getState().voice?.phase).toBe("listening");
  });

  it("routes levels by source and clamps to 0–1", () => {
    const s = useApp.getState();
    s.applyLevel({ source: "mic", level: 0.4 });
    s.applyLevel({ source: "speaker", level: 1.7 });
    expect(useApp.getState().micLevel).toBeCloseTo(0.4);
    expect(useApp.getState().speakerLevel).toBe(1);
    s.applyLevel({ source: "mic", level: Number.NaN });
    expect(useApp.getState().micLevel).toBe(0);
  });

  it("resets the speaker level when the character state changes", () => {
    const s = useApp.getState();
    s.setCharacter("talking");
    s.applyLevel({ source: "speaker", level: 0.5 });
    expect(useApp.getState().speakerLevel).toBe(0.5);
    s.setCharacter("idle");
    expect(useApp.getState().speakerLevel).toBeNull();
  });

  it("tracks downloads by model and keeps finished ones", () => {
    const s = useApp.getState();
    s.applyDownload({ model: "whisper:base.en", done_bytes: 10, total_bytes: 100, finished: false, error: null });
    s.applyDownload({ model: "piper:x", done_bytes: 1, total_bytes: null, finished: false, error: null });
    s.applyDownload({ model: "whisper:base.en", done_bytes: 100, total_bytes: 100, finished: true, error: null });
    const d = useApp.getState().downloads;
    expect(Object.keys(d).sort()).toEqual(["piper:x", "whisper:base.en"]);
    expect(d["whisper:base.en"]?.finished).toBe(true);
  });
});
