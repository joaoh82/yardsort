// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Reading a provisioning profile for the TestFlight workflow, and the export options made from it.
 * The profile is built as Apple's are, a property list between binary envelope bytes; no real
 * profile can be committed, as it names a real team.
 */

import { describe, expect, test } from "bun:test";

import { exportOptions, readProfile } from "../../scripts/ios-signing";

function profile(extra = "", name = "Yardsort App Store &amp; CI"): Uint8Array {
  const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AppIDName</key>
	<string>Yardsort</string>
	<key>Entitlements</key>
	<dict>
		<key>application-identifier</key>
		<string>ABCDE12345.dev.yardsort.mobile</string>
		<key>get-task-allow</key>
		<false/>
	</dict>
	<key>ExpirationDate</key>
	<date>2027-11-06T10:00:00Z</date>
	<key>Name</key>
	<string>${name}</string>${extra}
	<key>TeamIdentifier</key>
	<array>
		<string>ABCDE12345</string>
	</array>
	<key>TeamName</key>
	<string>Example Team</string>
	<key>UUID</key>
	<string>11111111-2222-3333-4444-555555555555</string>
</dict>
</plist>`;
  const text = new TextEncoder().encode(plist);
  const head = new Uint8Array([0x30, 0x82, 0x1f, 0x00, 0x06, 0x09, 0xff, 0xfe]);
  const tail = new Uint8Array([0xa0, 0x82, 0x01, 0x00, 0xff]);
  return new Uint8Array([...head, ...text, ...tail]);
}

describe("readProfile", () => {
  test("reads an App Store profile's name, UUID, team, app and expiry", () => {
    expect(readProfile(profile())).toEqual({
      name: "Yardsort App Store & CI",
      uuid: "11111111-2222-3333-4444-555555555555",
      teamId: "ABCDE12345",
      bundleId: "dev.yardsort.mobile",
      expires: "2027-11-06T10:00:00Z",
    });
  });

  test("keeps a name that is not ASCII", () => {
    expect(readProfile(profile("", "Yardsort – TestFlight")).name).toBe("Yardsort – TestFlight");
  });

  test("refuses a development or ad hoc profile", () => {
    const devices =
      "\n\t<key>ProvisionedDevices</key>\n\t<array>\n\t\t<string>00008110</string>\n\t</array>";
    expect(() => readProfile(profile(devices))).toThrow(/App Store one/);
  });

  test("refuses what is not a profile", () => {
    expect(() => readProfile(new TextEncoder().encode("-----BEGIN PRIVATE KEY-----"))).toThrow(
      /not a provisioning profile/,
    );
  });
});

describe("exportOptions", () => {
  test("exports for internal testers only, signed by hand with that profile", () => {
    const options = exportOptions(readProfile(profile()));
    expect(options).toContain("<key>method</key>\n  <string>app-store-connect</string>");
    expect(options).toContain("<key>destination</key>\n  <string>export</string>");
    expect(options).toContain("<key>testFlightInternalTestingOnly</key>\n  <true/>");
    expect(options).toContain("<key>teamID</key>\n  <string>ABCDE12345</string>");
    expect(options).toContain(
      "<key>dev.yardsort.mobile</key>\n    <string>Yardsort App Store &amp; CI</string>",
    );
  });
});
