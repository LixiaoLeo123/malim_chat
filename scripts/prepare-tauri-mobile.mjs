import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const gradlePath = resolve("apps/client/src-tauri/gen/android/app/build.gradle.kts");

if (existsSync(gradlePath)) {
  const source = readFileSync(gradlePath, "utf8");
  const updated = source.replace(
    'manifestPlaceholders["usesCleartextTraffic"] = "false"',
    'manifestPlaceholders["usesCleartextTraffic"] = "true"',
  );

  if (updated === source && !source.includes('manifestPlaceholders["usesCleartextTraffic"] = "true"')) {
    throw new Error("Could not configure Android cleartext traffic policy.");
  }
  if (updated !== source) writeFileSync(gradlePath, updated);
}
