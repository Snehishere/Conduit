import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

class ClipboardItem {
  final String content;
  final String mime;
  final String sourceDevice;
  final int timestamp;

  ClipboardItem({
    required this.content,
    required this.mime,
    required this.sourceDevice,
    required this.timestamp,
  });
}

typedef ClipboardChangeCallback = void Function(String content, String mime);

class ClipboardService extends ChangeNotifier {
  final List<ClipboardItem> _history = [];
  bool _isSyncing = false;
  bool _isListening = false;
  Timer? _pollTimer;
  String? _lastContent;
  ClipboardChangeCallback? _onChange;

  static const Duration _pollInterval = Duration(seconds: 2);

  List<ClipboardItem> get history => List.unmodifiable(_history);
  bool get isSyncing => _isSyncing;
  bool get isListening => _isListening;

  void startListening({ClipboardChangeCallback? onChange}) {
    if (_isListening) return;
    _isListening = true;
    _onChange = onChange;

    // Capture current clipboard content as baseline
    Clipboard.getData('text/plain').then((data) {
      _lastContent = data?.text;
    });

    _pollTimer = Timer.periodic(_pollInterval, (_) => _checkClipboard());
    notifyListeners();
  }

  void stopListening() {
    _pollTimer?.cancel();
    _pollTimer = null;
    _isListening = false;
    _onChange = null;
    notifyListeners();
  }

  Future<void> _checkClipboard() async {
    try {
      final data = await Clipboard.getData('text/plain');
      final currentContent = data?.text;

      if (currentContent != null &&
          currentContent.isNotEmpty &&
          currentContent != _lastContent) {
        _lastContent = currentContent;

        // Add to local history
        _addToHistory(currentContent, 'text/plain', 'mobile');

        // Notify listener (WebSocket sync)
        _onChange?.call(currentContent, 'text/plain');

        notifyListeners();
      }
    } catch (e) {
      debugPrint('Clipboard poll error: $e');
    }
  }

  Future<void> copyToClipboard(String text) async {
    await Clipboard.setData(ClipboardData(text: text));
    _lastContent = text;
    _addToHistory(text, 'text/plain', 'mobile');
    notifyListeners();
  }

  Future<String?> pasteFromClipboard() async {
    final data = await Clipboard.getData('text/plain');
    return data?.text;
  }

  void syncFromDevice(String content, String mime, String deviceId) {
    // Avoid echo — don't add if we just sent this
    if (content == _lastContent) return;

    _lastContent = content;
    _addToHistory(content, mime, deviceId);

    // Write to system clipboard so it's available for paste
    Clipboard.setData(ClipboardData(text: content));

    notifyListeners();
  }

  void _addToHistory(String content, String mime, String sourceDevice) {
    // Deduplicate — don't add consecutive identical entries
    if (_history.isNotEmpty && _history.first.content == content) return;

    _history.insert(
      0,
      ClipboardItem(
        content: content,
        mime: mime,
        sourceDevice: sourceDevice,
        timestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
      ),
    );

    if (_history.length > 50) {
      _history.removeLast();
    }
  }

  void clearHistory() {
    _history.clear();
    notifyListeners();
  }

  @override
  void dispose() {
    stopListening();
    super.dispose();
  }
}
