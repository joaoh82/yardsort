// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Sign the generated iOS project with an App Store provisioning profile, for a TestFlight build
 * made without Xcode's own account: `.github/workflows/testflight.yml` runs it after
 * `expo prebuild`. It installs the profile where Xcode looks, sets the app's target (Release only)
 * to sign by hand with it, as EAS does, and writes the options for `xcodebuild -exportArchive`: an
 * App Store Connect build, for internal testers only, exported to a file for `altool` to upload.
 *
 * Run with `bun`: `bun scripts/ios-signing.ts <profile.mobileprovision> <ExportOptions.plist>`.
 * Everything it needs is read from the profile, so the team and the profile's name are no secrets
 * of their own.
 */

import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import { IOSConfig } from "expo/config-plugins";

const ROOT = join(import.meta.dir, "..");

/** What a profile says about itself, of what signing needs. */
export type Profile = {
  name: string;
  uuid: string;
  teamId: string;
  bundleId: string;
  expires: string;
};

const ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&apos;": "'",
};

function unescapeXml(value: string): string {
  return value.replace(/&(amp|lt|gt|quot|apos);/g, (entity) => ENTITIES[entity] ?? entity);
}

function escapeXml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

/**
 * Read a `.mobileprovision`. It is a signed envelope with the property list inside it as plain text,
 * so the list is found by its first and last lines rather than by opening the envelope, which only
 * macOS's `security` does. A profile for development or ad hoc is refused, saying so: TestFlight
 * takes App Store builds only.
 */
export function readProfile(bytes: Uint8Array): Profile {
  const text = new TextDecoder().decode(bytes);
  const start = text.indexOf("<?xml");
  const end = text.indexOf("</plist>", start);
  if (start < 0 || end < 0)
    throw new Error("This is not a provisioning profile: it holds no property list.");
  const plist = text.slice(start, end);

  const string = (key: string): string | undefined => {
    const found = plist.match(new RegExp(`<key>${key}</key>\\s*<string>([^<]*)</string>`));
    return found?.[1] === undefined ? undefined : unescapeXml(found[1]);
  };
  const name = string("Name");
  const uuid = string("UUID");
  const teamId = plist.match(
    /<key>TeamIdentifier<\/key>\s*<array>\s*<string>([^<]*)<\/string>/,
  )?.[1];
  const appId = string("application-identifier");
  const expires = plist.match(/<key>ExpirationDate<\/key>\s*<date>([^<]*)<\/date>/)?.[1];
  if (!name || !uuid || !teamId || !appId || !expires) {
    throw new Error("This provisioning profile lacks its name, UUID, team, app ID or expiry date.");
  }
  if (
    /<key>ProvisionedDevices<\/key>/.test(plist) ||
    /<key>ProvisionsAllDevices<\/key>/.test(plist)
  ) {
    throw new Error(
      `"${name}" is a development, ad hoc or enterprise profile. TestFlight needs an App Store one: ` +
        "in the Apple Developer site, Profiles, +, App Store Connect.",
    );
  }
  if (!appId.startsWith(`${teamId}.`))
    throw new Error(`"${name}" names the app ${appId}, outside its own team.`);
  return { name, uuid, teamId, bundleId: appId.slice(teamId.length + 1), expires };
}

/** The options `xcodebuild -exportArchive` takes: what Xcode's TestFlight Internal Only would do. */
export function exportOptions(profile: Profile): string {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key>
  <string>app-store-connect</string>
  <key>destination</key>
  <string>export</string>
  <key>testFlightInternalTestingOnly</key>
  <true/>
  <key>teamID</key>
  <string>${escapeXml(profile.teamId)}</string>
  <key>signingStyle</key>
  <string>manual</string>
  <key>signingCertificate</key>
  <string>Apple Distribution</string>
  <key>provisioningProfiles</key>
  <dict>
    <key>${escapeXml(profile.bundleId)}</key>
    <string>${escapeXml(profile.name)}</string>
  </dict>
</dict>
</plist>
`;
}

function main(profilePath: string, optionsPath: string): void {
  const bytes = readFileSync(profilePath);
  const profile = readProfile(bytes);
  const bundleId = IOSConfig.BundleIdentifier.getBundleIdentifierFromPbxproj(ROOT);
  if (bundleId !== profile.bundleId) {
    throw new Error(
      `"${profile.name}" is for ${profile.bundleId}, but the project builds ${bundleId}. ` +
        "Was it prebuilt with APP_VARIANT=release?",
    );
  }

  // Xcode 16 and later look in the first, earlier ones in the second.
  for (const dir of [
    join(homedir(), "Library/Developer/Xcode/UserData/Provisioning Profiles"),
    join(homedir(), "Library/MobileDevice/Provisioning Profiles"),
  ]) {
    mkdirSync(dir, { recursive: true });
    copyFileSync(profilePath, join(dir, `${profile.uuid}.mobileprovision`));
  }

  IOSConfig.ProvisioningProfile.setProvisioningProfileForPbxproj(ROOT, {
    profileName: profile.name,
    appleTeamId: profile.teamId,
    codeSignIdentity: "Apple Distribution",
  });
  writeFileSync(optionsPath, exportOptions(profile));
  console.log(
    `Signing ${profile.bundleId} with "${profile.name}" (team ${profile.teamId}), valid until ${profile.expires}.`,
  );
}

if (import.meta.main) {
  const [profilePath, optionsPath] = process.argv.slice(2);
  if (!profilePath || !optionsPath) {
    console.error(
      "usage: bun scripts/ios-signing.ts <profile.mobileprovision> <ExportOptions.plist>",
    );
    process.exit(2);
  }
  main(profilePath, optionsPath);
}
