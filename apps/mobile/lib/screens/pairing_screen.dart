import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/pairing_service.dart';
import '../services/websocket_service.dart';
import '../theme/app_theme.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

class PairingScreen extends StatefulWidget {
  final String? ip;
  final int? port;

  const PairingScreen({super.key, this.ip, this.port});

  @override
  State<PairingScreen> createState() => _PairingScreenState();
}

class _PairingScreenState extends State<PairingScreen> {
  final _codeController = TextEditingController();
  bool _showManualEntry = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<PairingService>().requestCameraPermission();
    });
  }

  @override
  void dispose() {
    _codeController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Pair device'),
        actions: [
          TextButton(
            onPressed: () => setState(() => _showManualEntry = !_showManualEntry),
            child: Text(_showManualEntry ? 'Scan QR code' : 'Enter code'),
          ),
        ],
      ),
      body: Consumer<PairingService>(
        builder: (_, svc, _) {
          if (svc.state == PairingState.connected) {
            return _buildSuccess(svc, colors);
          }
          if (svc.state == PairingState.failed) {
            return _buildError(svc, colors);
          }
          if (svc.state == PairingState.connecting) {
            return _buildConnecting(colors);
          }
          return _showManualEntry
              ? _buildManualEntry(svc, colors)
              : _buildQrScanner(svc, colors);
        },
      ),
    );
  }

  Widget _buildQrScanner(PairingService svc, AppColors colors) {
    if (!svc.hasCameraPermission) {
      return Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(Icons.camera_alt, size: 64, color: colors.text3),
            const SizedBox(height: 16),
            Text(
              'Camera permission required',
              style: TextStyle(fontSize: 16, color: colors.text2),
            ),
            const SizedBox(height: 8),
            Text(
              'Allow Conduit to access your camera\nto scan QR codes',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 13, color: colors.text3),
            ),
            const SizedBox(height: 24),
            ElevatedButton(
              onPressed: () => svc.requestCameraPermission(),
              child: const Text('Grant permission'),
            ),
          ],
        ),
      );
    }

    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Container(
            width: 250,
            height: 250,
            decoration: BoxDecoration(
              border: Border.all(color: colors.accentSecondary, width: 2),
              borderRadius: BorderRadius.circular(16),
            ),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(14),
              child: MobileScanner(
                onDetect: (capture) {
                  final List<Barcode> barcodes = capture.barcodes;
                  if (barcodes.isNotEmpty) {
                    final String? qrData = barcodes.first.rawValue;
                    if (qrData != null && svc.state == PairingState.idle) {
                      final wsService = context.read<WebSocketService>();
                      svc.pairFromQr(qrData, wsService);
                    }
                  }
                },
              ),
            ),
          ),
          const SizedBox(height: 24),
          Text(
            'Open Conduit on your desktop and\nscan the pairing QR code',
            textAlign: TextAlign.center,
            style: TextStyle(fontSize: 13, color: colors.text3),
          ),
        ],
      ),
    );
  }

  Widget _buildManualEntry(PairingService svc, AppColors colors) {
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.keyboard, size: 64, color: colors.text3),
          const SizedBox(height: 16),
          Text(
            'Enter pairing code',
            style: TextStyle(fontSize: 16, color: colors.text2),
          ),
          const SizedBox(height: 8),
          Text(
            'Type the code shown on your desktop',
            style: TextStyle(fontSize: 13, color: colors.text3),
          ),
          const SizedBox(height: 24),
          TextField(
            controller: _codeController,
            textAlign: TextAlign.center,
            style: TextStyle(fontSize: 20, letterSpacing: 4, color: colors.text1),
            decoration: InputDecoration(
              hintText: 'XXXX-XXXX-XXXX',
              hintStyle: TextStyle(color: colors.text3),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(12),
                borderSide: BorderSide(color: colors.border),
              ),
              enabledBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(12),
                borderSide: BorderSide(color: colors.border),
              ),
              focusedBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(12),
                borderSide: BorderSide(color: colors.accent, width: 1.5),
              ),
            ),
            keyboardType: TextInputType.text,
            textCapitalization: TextCapitalization.characters,
          ),
          const SizedBox(height: 16),
          SizedBox(
            width: double.infinity,
            child: ElevatedButton(
              onPressed: () {
                final code = _codeController.text.trim();
                if (code.isNotEmpty) {
                  final wsService = context.read<WebSocketService>();
                  svc.pairFromCode(code, wsService, ip: widget.ip, port: widget.port);
                }
              },
              child: const Text('Connect'),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildConnecting(AppColors colors) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          CircularProgressIndicator(color: colors.accentSecondary),
          const SizedBox(height: 24),
          Text('Connecting...', style: TextStyle(fontSize: 16, color: colors.text1)),
          const SizedBox(height: 8),
          Text(
            'Opening a TLS connection to the desktop',
            style: TextStyle(fontSize: 13, color: colors.text2),
          ),
        ],
      ),
    );
  }

  Widget _buildSuccess(PairingService svc, AppColors colors) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Container(
            width: 80,
            height: 80,
            decoration: BoxDecoration(
              color: colors.accent,
              shape: BoxShape.circle,
            ),
            child: const Icon(Icons.check, color: Colors.white, size: 40),
          ),
          const SizedBox(height: 24),
          Text(
            'Paired',
            style: TextStyle(fontSize: 24, fontWeight: FontWeight.bold, color: colors.text1),
          ),
          const SizedBox(height: 8),
          Text(
            'Connected to ${svc.pairedDevice?.deviceName ?? "desktop"}',
            style: TextStyle(fontSize: 14, color: colors.text2),
          ),
          const SizedBox(height: 32),
          ElevatedButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Done'),
          ),
        ],
      ),
    );
  }

  Widget _buildError(PairingService svc, AppColors colors) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.error_outline, size: 64, color: colors.error),
          const SizedBox(height: 16),
          Text(
            svc.error ?? 'Pairing failed',
            textAlign: TextAlign.center,
            style: TextStyle(fontSize: 14, color: colors.text1),
          ),
          const SizedBox(height: 24),
          ElevatedButton(
            onPressed: () => svc.reset(),
            child: const Text('Try again'),
          ),
        ],
      ),
    );
  }
}
