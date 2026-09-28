// ignore_for_file: experimental_member_use

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';
import 'package:flutter/foundation.dart';
import 'package:record/record.dart';
import 'package:just_audio/just_audio.dart';
import 'package:audio_session/audio_session.dart';

/// Audio streaming service for capturing and playing audio.
/// Supports duplex (simultaneous capture + playback) via WebSocket.
class AudioStreamService extends ChangeNotifier {
  AudioRecorder? _recorder;
  AudioPlayer? _player;
  bool _isStreaming = false;
  bool _isPlaying = false;
  bool _isDuplexMode = false;
  StreamSubscription<Uint8List>? _audioSubscription;
  StreamSubscription<PlayerState>? _playerStateSub;
  void Function(Map<String, dynamic>)? _sendMessage;

  bool get isStreaming => _isStreaming;
  bool get isPlaying => _isPlaying;
  bool get isDuplexMode => _isDuplexMode;

  void setSendFunction(void Function(Map<String, dynamic>) sendFn) {
    _sendMessage = sendFn;
  }

  /// Initialize audio session for recording and playback
  Future<void> initialize() async {
    try {
      final session = await AudioSession.instance;
      await session.configure(const AudioSessionConfiguration(
        avAudioSessionCategory: AVAudioSessionCategory.playAndRecord,
        avAudioSessionMode: AVAudioSessionMode.voiceChat,
        androidAudioAttributes: AndroidAudioAttributes(
          contentType: AndroidAudioContentType.speech,
          usage: AndroidAudioUsage.voiceCommunication,
        ),
      ));
      debugPrint('Audio session configured for voice communication');
    } catch (e) {
      debugPrint('Failed to initialize audio session: $e');
    }
  }

  /// Start capturing audio and streaming via WebSocket (mic -> desktop)
  Future<void> startStreaming() async {
    if (_isStreaming) return;

    try {
      _recorder = AudioRecorder();

      // Check and request permissions
      if (!await _recorder!.hasPermission()) {
        debugPrint('Audio recording permission not granted');
        return;
      }

      // Start recording with PCM format for raw audio streaming
      const recordConfig = RecordConfig(
        encoder: AudioEncoder.pcm16bits,
        sampleRate: 16000,
        numChannels: 1,
      );

      final stream = await _recorder!.startStream(recordConfig);

      _sendMessage?.call({
        'type': 'audio',
        'action': 'stream_start',
        'format': 'pcm16',
        'sample_rate': 16000,
        'channels': 1,
      });

      _audioSubscription = stream.listen(
        (audioData) {
          // Send audio chunk via WebSocket
          _sendMessage?.call({
            'type': 'audio',
            'action': _isDuplexMode ? 'playback_data' : 'stream_data',
            'data': base64Encode(audioData),
            'format': 'pcm16',
            'sample_rate': 16000,
            'channels': 1,
          });
        },
        onError: (error) {
          debugPrint('Audio stream error: $error');
        },
      );

      _isStreaming = true;
      notifyListeners();
      debugPrint('Audio streaming started (duplex=$_isDuplexMode)');
    } catch (e) {
      debugPrint('Failed to start audio streaming: $e');
    }
  }

  /// Stop audio streaming
  Future<void> stopStreaming() async {
    if (!_isStreaming) return;

    try {
      await _audioSubscription?.cancel();
      _audioSubscription = null;
      await _recorder?.stop();
      _recorder = null;
      _isStreaming = false;
      notifyListeners();
      debugPrint('Audio streaming stopped');
    } catch (e) {
      debugPrint('Failed to stop audio streaming: $e');
    }
  }

  /// Enter duplex mode: mobile captures mic and desktop captures system audio
  /// simultaneously, with both streams flowing in both directions.
  Future<void> startDuplexMode() async {
    if (_isDuplexMode) return;

    _isDuplexMode = true;

    // Tell desktop to start its persistent playback output stream
    _sendMessage?.call({
      'type': 'audio',
      'action': 'playback_start',
    });

    // Start mic capture if not already streaming
    if (!_isStreaming) {
      await startStreaming();
    }

    notifyListeners();
    debugPrint('Duplex mode enabled');
  }

  /// Exit duplex mode: stop sending mic data as playback_data,
  /// keep regular streaming if desired.
  Future<void> stopDuplexMode() async {
    if (!_isDuplexMode) return;

    _isDuplexMode = false;

    // Tell desktop to stop its persistent playback output stream
    _sendMessage?.call({
      'type': 'audio',
      'action': 'playback_stop',
    });

    notifyListeners();
    debugPrint('Duplex mode disabled');
  }

  /// Play received audio data (from desktop mic/system audio -> mobile speaker)
  Future<void> playAudioData(String base64Data, {String format = 'pcm16', int sampleRate = 16000}) async {
    try {
      final audioBytes = base64Decode(base64Data);

      // Decode PCM16 to float32 for just_audio
      final pcm16 = Int16List.view(audioBytes.buffer);
      final float32 = Float32List(pcm16.length);
      for (var i = 0; i < pcm16.length; i++) {
        float32[i] = pcm16[i] / 32768.0;
      }

      // Create audio player and play the data
      _player ??= AudioPlayer();
      await _player!.setAudioSource(
        _CustomAudioSource(float32, sampleRate),
      );
      await _player!.play();
      _isPlaying = true;
      notifyListeners();

      // Listen for completion (single subscription — reuse across chunks)
      _playerStateSub ??= _player!.playerStateStream.listen((state) {
        if (state.processingState == ProcessingState.completed) {
          _isPlaying = false;
          notifyListeners();
        }
      });
    } catch (e) {
      debugPrint('Failed to play audio: $e');
    }
  }

  /// Stop playback
  Future<void> stopPlayback() async {
    try {
      await _player?.stop();
      _isPlaying = false;
      notifyListeners();
    } catch (e) {
      debugPrint('Failed to stop playback: $e');
    }
  }

  @override
  void dispose() {
    stopStreaming();
    stopDuplexMode();
    stopPlayback();
    _playerStateSub?.cancel();
    _audioSubscription?.cancel();
    _player?.dispose();
    super.dispose();
  }
}

/// Custom audio source for playing raw PCM data
class _CustomAudioSource extends StreamAudioSource {
  final Float32List _pcmData;
  final int sampleRate;

  _CustomAudioSource(this._pcmData, this.sampleRate);

  @override
  Future<StreamAudioResponse> request([int? start, int? end]) async {
    start ??= 0;
    end ??= _pcmData.length * 2; // 2 bytes per sample (16-bit)

    final byteData = ByteData.view(_pcmData.buffer);
    final bytes = Uint8List.view(byteData.buffer);
    final range = bytes.sublist(
      start.clamp(0, bytes.length),
      end.clamp(0, bytes.length),
    );

    return StreamAudioResponse(
      sourceLength: bytes.length,
      contentLength: range.length,
      offset: start,
      stream: Stream.value(range),
      contentType: 'audio/pcm',
    );
  }
}
