import { looksLikeLan } from "./SettingsPanel";

describe("looksLikeLan", () => {
  it("matches private ranges and local names", () => {
    for (const h of ["192.168.1.5", "10.0.0.2", "172.20.0.1", "100.100.1.1", "box.local", "gpu.lan", "fd00::1"]) {
      expect(looksLikeLan(h), h).toBe(true);
    }
    for (const h of ["localhost", "127.0.0.1", "api.anthropic.com", "172.32.0.1", "8.8.8.8"]) {
      expect(looksLikeLan(h), h).toBe(false);
    }
  });
});
