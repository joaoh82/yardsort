// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import ExpoModulesCore

/// The view that hears a keyboard paired with the phone; see `src/lib/hardware.ts` for what it takes.
public final class HardwareKeysModule: Module {
  public func definition() -> ModuleDefinition {
    Name("HardwareKeys")

    View(HardwareKeysView.self) {
      Events("onKey")

      Prop("enabled") { (view, enabled: Bool) in
        view.setTaking(enabled)
      }

      Prop("plain") { (view, plain: Bool) in
        view.plain = plain
      }
    }
  }
}
