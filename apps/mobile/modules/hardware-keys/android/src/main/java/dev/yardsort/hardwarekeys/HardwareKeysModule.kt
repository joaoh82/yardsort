// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

package dev.yardsort.hardwarekeys

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/** The view that hears a keyboard plugged into the phone; see `src/lib/hardware.ts` for what it takes. */
class HardwareKeysModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("HardwareKeys")

    View(HardwareKeysView::class) {
      Events("onKey")

      Prop("enabled") { view: HardwareKeysView, enabled: Boolean ->
        view.setTaking(enabled)
      }

      Prop("plain") { view: HardwareKeysView, plain: Boolean ->
        view.plain = plain
      }
    }
  }
}
