import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:conduit/services/native_screen_capture.dart';

/// Walks up from the test file to the `apps/mobile` package root.
Directory _packageRoot() {
  var dir = Directory.current;
  while (!File('${dir.path}/pubspec.yaml').existsSync()) {
    final parent = dir.parent;
    if (parent.path == dir.path) {
      throw StateError('pubspec.yaml not found above ${Directory.current.path}');
    }
    dir = parent;
  }
  return dir;
}

/// The screen-mirror section of the iOS host, and nothing else.
///
/// `apps/mobile/ios/Runner/AppDelegate.swift` is the iOS capture producer. It
/// has no test target in this repository and `swiftc` cannot run on the CI
/// host, so the shape it emits is pinned here instead: these assertions fail
/// the moment the producer and the reader disagree again.
String _iosScreenMirrorSection() {
  final source = File(
    '${_packageRoot().path}/ios/Runner/AppDelegate.swift',
  ).readAsStringSync();
  final start = source.indexOf('// MARK: - Screen Mirror');
  final end = source.indexOf('// MARK: - Bluetooth');
  expect(start, greaterThanOrEqualTo(0),
      reason: 'AppDelegate.swift must keep a "Screen Mirror" MARK section');
  expect(end, greaterThan(start),
      reason: 'AppDelegate.swift must keep a "Bluetooth" MARK after it');
  return source.substring(start, end);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('the iOS producer emits the canonical frame shape the reader requires',
      () {
    final section = _iosScreenMirrorSection();

    // `packages/protocol/src/types.rs:594` — `ScreenMirrorFrame`.
    expect(section, contains('"type": "screen_mirror"'),
        reason: 'the type must be the protocol type, not a private variant');
    expect(section, contains('"action": "frame"'));
    expect(section, contains('"data":'),
        reason: 'the JPEG goes under `data`; `frame` is not a protocol field');
    expect(section, contains('"format": "jpeg"'));
    expect(section, contains('"width":'));
    expect(section, contains('"height":'));
  });

  test('the iOS producer no longer emits the unreadable legacy shape', () {
    // `screen_mirror_frame` with the payload under `frame` was what
    // `NativeScreenCapture.normalizeNativeFrame` calls the legacy iOS shape.
    // Android and the Dart reader both speak the canonical one.
    expect(_iosScreenMirrorSection(), isNot(contains('"type": "screen_mirror_frame"')));
  });

  test('the iOS producer honours the requested quality and frame rate', () {
    final section = _iosScreenMirrorSection();

    // A hardcoded width and compression quality means the user's Quality and
    // FPS controls do nothing on iOS. The desktop's own capture reads the same
    // two fields, and so does the Android capture pipeline.
    expect(section, contains('arguments?["quality"]'),
        reason: 'the quality argument must be read, not ignored');
    expect(section, contains('arguments?["fps"]'),
        reason: 'the fps argument must be read, not ignored');
    expect(section, contains('30.0'),
        reason: 'the requested rate must be clamped to the 30 fps the '
            'capture pipeline and the hub budget are sized for');
    expect(section, isNot(contains('maxWidth: CGFloat = 640')),
        reason: 'the hardcoded 640pt width is what the fix removes');
    expect(section, isNot(contains('compressionQuality: 0.5')),
        reason: 'the hardcoded compression quality is what the fix removes');
  });

  test('the reader understands both shapes the field may still produce', () {
    // Android emits the canonical shape; an iOS build predating this fix emits
    // the legacy one. The reader has to keep working for the second until it is
    // retired, or those users silently lose screen mirroring again.
    final canonical = NativeScreenCapture.normalizeNativeFrame(<String, Object?>{
      'type': 'screen_mirror',
      'action': 'frame',
      'data': 'AAECAw==',
      'width': 10,
      'height': 20,
    });
    expect(canonical, isNotNull);
    expect(canonical!['data'], 'AAECAw==');
    expect(canonical['width'], 10);

    final legacy = NativeScreenCapture.normalizeNativeFrame(<String, Object?>{
      'type': 'screen_mirror_frame',
      'frame': 'AAECAw==',
      'width': 10,
      'height': 20,
    });
    expect(legacy, isNotNull);
    expect(legacy!['data'], 'AAECAw==');
    expect(legacy['type'], 'screen_mirror');
    expect(legacy['action'], 'frame');
  });
}