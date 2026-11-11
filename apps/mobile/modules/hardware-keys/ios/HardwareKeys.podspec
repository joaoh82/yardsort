# This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
# of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

Pod::Spec.new do |s|
  s.name           = 'HardwareKeys'
  s.version        = '0.1.0'
  s.summary        = 'The keys a terminal needs from a keyboard paired with the phone'
  s.description    = 'Esc, Tab, the arrows, Ctrl and Alt from a hardware keyboard, for Yardsort'
  s.license        = 'MPL-2.0'
  s.author         = 'Yardsort'
  s.homepage       = 'https://github.com/joaoh82/yardsort'
  s.platforms      = { :ios => '15.1' }
  s.source         = { git: 'https://github.com/joaoh82/yardsort.git' }
  s.static_framework = true
  s.swift_version  = '5.9'

  s.dependency 'ExpoModulesCore'

  s.source_files = '**/*.{h,m,swift}'
end
