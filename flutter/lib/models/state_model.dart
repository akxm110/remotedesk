import 'dart:ui' show Rect;

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/utils/multi_window_manager.dart';
import 'package:get/get.dart';

import '../consts.dart';
import './platform_model.dart';

enum SvcStatus { notReady, connecting, ready }

class StateGlobal {
  int _windowId = -1;
  final RxBool _fullscreen = false.obs;
  bool _isMinimized = false;
  final RxBool isMaximized = false.obs;
  final RxBool _showTabBar = true.obs;
  final RxDouble _resizeEdgeSize = RxDouble(windowResizeEdgeSize);
  final RxDouble _windowBorderWidth = RxDouble(kWindowBorderWidth);
  final RxBool showRemoteToolBar = false.obs;
  final svcStatus = SvcStatus.notReady.obs;
  final RxInt videoConnCount = 0.obs;
  final RxBool isFocused = false.obs;
  // for mobile and web
  bool isInMainPage = true;
  bool isWebVisible = true;

  final isPortrait = false.obs;

  final updateUrl = ''.obs;

  String _inputSource = '';

  // Track relative mouse mode state for each peer connection.
  // Key: peerId, Value: true if relative mouse mode is active.
  // Note: This is session-only runtime state, NOT persisted to config.
  final RxMap<String, bool> relativeMouseModeState = <String, bool>{}.obs;

  // Use for desktop -> remote toolbar -> resolution
  final Map<String, Map<int, String?>> _lastResolutionGroupValues = {};

  int get windowId => _windowId;
  RxBool get fullscreen => _fullscreen;
  bool get isMinimized => _isMinimized;
  double get tabBarHeight => fullscreen.isTrue ? 0 : kDesktopRemoteTabBarHeight;
  RxBool get showTabBar => _showTabBar;
  RxDouble get resizeEdgeSize => _resizeEdgeSize;
  RxDouble get windowBorderWidth => _windowBorderWidth;

  resetLastResolutionGroupValues(String peerId) {
    _lastResolutionGroupValues[peerId] = {};
  }

  setLastResolutionGroupValue(
      String peerId, int currentDisplay, String? value) {
    if (!_lastResolutionGroupValues.containsKey(peerId)) {
      _lastResolutionGroupValues[peerId] = {};
    }
    _lastResolutionGroupValues[peerId]![currentDisplay] = value;
  }

  String? getLastResolutionGroupValue(String peerId, int currentDisplay) {
    return _lastResolutionGroupValues[peerId]?[currentDisplay];
  }

  setWindowId(int id) => _windowId = id;
  setMaximized(bool v) {
    if (!_fullscreen.isTrue) {
      if (isMaximized.value != v) {
        isMaximized.value = v;
        refreshResizeEdgeSize();
      }
      if (!isMacOS) {
        _windowBorderWidth.value = v ? 0 : kWindowBorderWidth;
      }
    }
  }

  setMinimized(bool v) => _isMinimized = v;

  setFullscreen(bool v, {bool procWnd = true}) {
    if (_fullscreen.value != v) {
      _fullscreen.value = v;
      _showTabBar.value = !_fullscreen.value;
      if (v) _hideMainWindowToTray();
      if (isWebDesktop) {
        procFullscreenWeb();
      } else if (!v && _spanRestoreFrame != null) {
        _exitSpanFullscreen();
      } else {
        procFullscreenNative(procWnd);
      }
    }
  }

  // Frame to restore after fullscreen that spans all local monitors.
  Rect? _spanRestoreFrame;
  // Center of the leftmost monitor as a fraction of the spanned window width,
  // set only while fullscreen spans all local monitors.
  final spanToolbarCenter = Rxn<double>();
  // The same for a window that is not fullscreen but still covers more than one monitor:
  // the center of its part on the leftmost one, so a centered toolbar avoids the seam.
  final seamToolbarCenter = Rxn<double>();

  Future<void> updateSeamToolbarCenter() async {
    if (isWebDesktop || isMainDesktopWindow) return;
    try {
      final frame = await WindowController.fromWindowId(windowId).getFrame();
      final parts = (await getScreenRectList())
          .map((r) => r.intersect(frame))
          .where((r) => r.width > 1 && r.height > 1)
          .toList();
      if (parts.length < 2 || frame.width <= 0) {
        seamToolbarCenter.value = null;
        return;
      }
      final leftmost = parts.reduce((a, b) => a.left <= b.left ? a : b);
      seamToolbarCenter.value = (leftmost.center.dx - frame.left) / frame.width;
    } catch (_) {
      // Leave the toolbar where it was.
    }
  }

  Future<void> setFullscreenAllMonitors() async {
    if (_fullscreen.isTrue || isWebDesktop) {
      return;
    }
    final rects = await getScreenRectList();
    if (rects.length < 2) {
      setFullscreen(true);
      return;
    }
    var union = rects.first;
    for (final r in rects.skip(1)) {
      union = union.expandToInclude(r);
    }
    final wc = WindowController.fromWindowId(windowId);
    _spanRestoreFrame = await wc.getFrame();
    final leftmost = rects.reduce((a, b) => a.left <= b.left ? a : b);
    if (union.width > 0) {
      spanToolbarCenter.value =
          (leftmost.center.dx - union.left) / union.width;
    }
    _fullscreen.value = true;
    _showTabBar.value = false;
    _hideMainWindowToTray();
    refreshResizeEdgeSize();
    _windowBorderWidth.value = 0;
    _spanViaPlugin = isWindows;
    if (_spanViaPlugin) {
      // Enter the plugin's own fullscreen first: only then does the window
      // frame stop insetting the client area, which left gaps at the edges.
      await wc.setFullscreen(true);
      await Future.delayed(const Duration(milliseconds: 200));
      if (bind.mainSetSpanFullscreen(enable: true)) return;
    }
    await wc.setFrame(union);
  }

  // Whether the span went through the plugin's fullscreen, which then also
  // restores the window.
  bool _spanViaPlugin = false;

  void _hideMainWindowToTray() {
    if (isDesktop && !isMainDesktopWindow) {
      rustDeskWinManager.call(WindowType.Main, kWindowEventHideMainToTray, '');
    }
  }

  _exitSpanFullscreen() {
    final frame = _spanRestoreFrame;
    _spanRestoreFrame = null;
    spanToolbarCenter.value = null;
    refreshResizeEdgeSize();
    _windowBorderWidth.value = kWindowBorderWidth;
    if (_spanViaPlugin) {
      _spanViaPlugin = false;
      bind.mainSetSpanFullscreen(enable: false);
      WindowController.fromWindowId(windowId).setFullscreen(false);
    } else if (frame != null) {
      WindowController.fromWindowId(windowId).setFrame(frame);
    }
  }

  procFullscreenWeb() {
    final isFullscreen = ffiGetByName('fullscreen') == 'Y';
    String fullscreenValue = '';
    if (isFullscreen && _fullscreen.isFalse) {
      fullscreenValue = 'N';
    } else if (!isFullscreen && fullscreen.isTrue) {
      fullscreenValue = 'Y';
    }
    if (fullscreenValue.isNotEmpty) {
      ffiSetByName('fullscreen', fullscreenValue);
    }
  }

  procFullscreenNative(bool procWnd) {
    refreshResizeEdgeSize();
    print("fullscreen: $fullscreen, resizeEdgeSize: ${_resizeEdgeSize.value}");
    _windowBorderWidth.value = fullscreen.isTrue ? 0 : kWindowBorderWidth;
    if (procWnd) {
      final wc = WindowController.fromWindowId(windowId);
      wc.setFullscreen(_fullscreen.isTrue).then((_) {
        // We remove the redraw (width + 1, height + 1), because this issue cannot be reproduced.
        // https://github.com/rustdesk/rustdesk/issues/9675
      });
    }
  }

  refreshResizeEdgeSize() => _resizeEdgeSize.value = fullscreen.isTrue
      ? kFullScreenEdgeSize
      : isMaximized.isTrue
          ? kMaximizeEdgeSize
          : windowResizeEdgeSize;

  String getInputSource({bool force = false}) {
    if (force || _inputSource.isEmpty) {
      _inputSource = bind.mainGetInputSource();
    }
    return _inputSource;
  }

  setInputSource(SessionID sessionId, String v) async {
    await bind.mainSetInputSource(sessionId: sessionId, value: v);
    _inputSource = bind.mainGetInputSource();
  }

  StateGlobal._() {
    if (isWebDesktop) {
      platformFFI.setFullscreenCallback((v) {
        _fullscreen.value = v;
      });
    }
  }

  static final StateGlobal instance = StateGlobal._();
}

// This final variable is initialized when the first time it is accessed.
final stateGlobal = StateGlobal.instance;
