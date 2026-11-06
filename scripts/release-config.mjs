// Write the Tauri config override for a release build, from the
// environment, to the path given as the argument:
//   UPDATER=1                  create signed updater packages; the public
//                              key (GLIDEGIT_UPDATER_PUBKEY) goes into the
//                              config, where the bundler and the app read it
//   WINDOWS_CERT_THUMBPRINT=…  sign the Windows build with this certificate
import { writeFileSync } from "node:fs";

const out = process.argv[2];
if (!out) {
  console.error("usage: node scripts/release-config.mjs <output.json>");
  process.exit(2);
}
const config = { bundle: {} };
if (process.env.UPDATER === "1") {
  const pubkey = (process.env.GLIDEGIT_UPDATER_PUBKEY ?? "").trim();
  if (!pubkey) throw new Error("UPDATER=1 needs GLIDEGIT_UPDATER_PUBKEY");
  config.bundle.createUpdaterArtifacts = true;
  config.plugins = { updater: { pubkey } };
}
if (process.env.WINDOWS_CERT_THUMBPRINT) {
  config.bundle.windows = { certificateThumbprint: process.env.WINDOWS_CERT_THUMBPRINT };
}
writeFileSync(out, JSON.stringify(config, null, 2));
console.log(JSON.stringify(config));
