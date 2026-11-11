// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

import ExpoModulesCore
import UIKit

/// Key commands for what a terminal needs from a hardware keyboard, around the composer. UIKit asks
/// every responder from the first one up for its commands, so these are heard while the composer's
/// field is being typed into, and, with `wantsPriorityOverSystemBehavior`, before the field moves
/// its cursor with an arrow or takes Ctrl-A as the start of the line. While no field is, this view
/// is the first responder itself, so Esc and Ctrl-C work without tapping the field first. A view
/// that is no text input brings up no keyboard on the screen.
///
/// What is taken is kept in step with `takes` in `src/lib/hardware.ts`.
public final class HardwareKeysView: ExpoView {
  let onKey = EventDispatcher()

  private var enabled = false
  /// Whether the arrows, Tab and Backspace on their own are taken: while the field is empty.
  var plain = false

  private static let ctrlChars = Array("abcdefghijklmnopqrstuvwxyz[\\] ").map(String.init)
  private static let altChars = Array("abcdefghijklmnopqrstuvwxyz0123456789`-=[]\\;',./").map(String.init)
  private static let letters = Array("abcdefghijklmnopqrstuvwxyz").map(String.init)
  private static let arrows: [String: String] = [
    UIKeyCommand.inputUpArrow: "up",
    UIKeyCommand.inputDownArrow: "down",
    UIKeyCommand.inputLeftArrow: "left",
    UIKeyCommand.inputRightArrow: "right",
  ]
  private static let tab = "\t"
  private static let backspace = "\u{8}"

  /// Taken whatever the field holds.
  private lazy var always: [UIKeyCommand] = {
    var commands = [self.command(UIKeyCommand.inputEscape, [])]
    for input in Self.arrows.keys {
      for flags: UIKeyModifierFlags in [
        .shift, .alternate, .control, [.shift, .alternate], [.shift, .control],
        [.alternate, .control], [.shift, .alternate, .control],
      ] {
        commands.append(self.command(input, flags))
      }
    }
    commands.append(self.command(Self.tab, .shift))
    for flags: UIKeyModifierFlags in [.alternate, .control, [.alternate, .control]] {
      commands.append(self.command(Self.backspace, flags))
    }
    for char in Self.ctrlChars {
      commands.append(self.command(char, .control))
      commands.append(self.command(char, [.control, .shift]))
      commands.append(self.command(char, [.control, .alternate]))
    }
    for char in Self.altChars {
      commands.append(self.command(char, .alternate))
    }
    for char in Self.letters {
      commands.append(self.command(char, [.alternate, .shift]))
      commands.append(self.command(char.uppercased(), [.alternate, .shift]))
    }
    return commands
  }()

  /// Taken only while the field is empty.
  private lazy var plainOnly: [UIKeyCommand] = {
    Self.arrows.keys.map { self.command($0, []) }
      + [self.command(Self.tab, []), self.command(Self.backspace, [])]
  }()

  public required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    for name in [UITextView.textDidEndEditingNotification, UITextField.textDidEndEditingNotification] {
      NotificationCenter.default.addObserver(
        self, selector: #selector(fieldLeft), name: name, object: nil)
    }
  }

  deinit {
    NotificationCenter.default.removeObserver(self)
  }

  func setTaking(_ enabled: Bool) {
    self.enabled = enabled
    if enabled {
      takeKeysSoon()
    } else if isFirstResponder {
      resignFirstResponder()
    }
  }

  public override var canBecomeFirstResponder: Bool { enabled }

  public override var keyCommands: [UIKeyCommand]? {
    guard enabled else { return nil }
    return plain ? always + plainOnly : always
  }

  public override func didMoveToWindow() {
    super.didMoveToWindow()
    if window != nil { takeKeysSoon() }
  }

  @objc private func fieldLeft() {
    takeKeysSoon()
  }

  /// Become the first responder once nothing else is, as after the field was left.
  private func takeKeysSoon() {
    DispatchQueue.main.async { [weak self] in
      guard let self, self.enabled, self.window != nil, !self.isFirstResponder else { return }
      if FirstResponder.find() == nil { self.becomeFirstResponder() }
    }
  }

  private func command(_ input: String, _ flags: UIKeyModifierFlags) -> UIKeyCommand {
    let command = UIKeyCommand(input: input, modifierFlags: flags, action: #selector(pressed(_:)))
    command.wantsPriorityOverSystemBehavior = true
    return command
  }

  @objc private func pressed(_ command: UIKeyCommand) {
    guard enabled, let input = command.input else { return }
    let key: String
    if let arrow = Self.arrows[input] {
      key = arrow
    } else if input == UIKeyCommand.inputEscape {
      key = "escape"
    } else if input == Self.tab {
      key = "tab"
    } else if input == Self.backspace {
      key = "backspace"
    } else {
      key = input.lowercased()
    }
    let flags = command.modifierFlags
    onKey([
      "key": key,
      "ctrl": flags.contains(.control),
      "alt": flags.contains(.alternate),
      "shift": flags.contains(.shift),
    ])
  }
}

/// The current first responder, found by sending an action to whichever it is.
@MainActor
private enum FirstResponder {
  static weak var found: UIResponder?

  static func find() -> UIResponder? {
    found = nil
    _ = UIApplication.shared.sendAction(
      #selector(UIResponder.yardsortFoundFirstResponder(_:)), to: nil, from: nil, for: nil)
    return found
  }
}

extension UIResponder {
  @objc fileprivate func yardsortFoundFirstResponder(_ sender: Any?) {
    FirstResponder.found = self
  }
}
