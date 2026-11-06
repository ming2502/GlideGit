// Check an update package's signature against the updater public key, the
// way the in-app updater does (minisign: Ed25519 over the BLAKE2b-512 hash
// of the file, plus a signature over the trusted comment). Run by the
// release workflow before anything is published: a private key on GitHub
// that does not match the public key in the app would make every later
// update fail.
//
//   node scripts/verify-update-sig.mjs <file> <file.sig> <pubkey-file>
//
// The .sig and the public key are as Tauri writes them: base64 of the
// minisign text.
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";

const [file, sigFile, keyFile] = process.argv.slice(2);
if (!file || !sigFile || !keyFile) {
  console.error("usage: node scripts/verify-update-sig.mjs <file> <file.sig> <pubkey-file>");
  process.exit(2);
}

const unwrap = (p) => Buffer.from(readFileSync(p, "utf8").trim(), "base64").toString("utf8").split("\n");
const fail = (msg) => {
  console.error(`signature check failed: ${msg}`);
  process.exit(1);
};

// Public key: "untrusted comment: …", base64("Ed" + key id (8) + key (32)).
const keyLines = unwrap(keyFile);
const key = Buffer.from(keyLines[1] ?? "", "base64");
if (key.length !== 42 || key.subarray(0, 2).toString() !== "Ed") fail("not a minisign public key");
const keyId = key.subarray(2, 10);
const publicKey = createPublicKey({
  key: { kty: "OKP", crv: "Ed25519", x: key.subarray(10).toString("base64url") },
  format: "jwk",
});

// Signature: "untrusted comment: …", base64("ED" + key id + sig (64)),
// "trusted comment: …", base64(global sig (64)).
const sigLines = unwrap(sigFile);
const sig = Buffer.from(sigLines[1] ?? "", "base64");
const trusted = (sigLines[2] ?? "").replace(/^trusted comment: /, "");
const globalSig = Buffer.from(sigLines[3] ?? "", "base64");
if (sig.length !== 74) fail("not a minisign signature");
if (sig.subarray(0, 2).toString() !== "ED") fail("expected a prehashed (ED) signature");
if (!sig.subarray(2, 10).equals(keyId)) {
  fail(`made with key ${sig.subarray(2, 10).toString("hex")}, the public key is ${keyId.toString("hex")}`);
}
const digest = createHash("blake2b512").update(readFileSync(file)).digest();
if (!verify(null, digest, publicKey, sig.subarray(10))) fail("the file does not match its signature");
if (!verify(null, Buffer.concat([sig.subarray(10), Buffer.from(trusted)]), publicKey, globalSig)) {
  fail("the trusted comment does not match its signature");
}
console.log(`ok: ${file} is signed by key ${keyId.toString("hex")} (${trusted})`);
