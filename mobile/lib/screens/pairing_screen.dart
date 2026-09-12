import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/pairing_service.dart';
import '../services/websocket_service.dart';

class PairingScreen extends StatefulWidget {
  const PairingScreen({super.key});

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
    return Scaffold(
      appBar: AppBar(
        title: const Text('Pair Device'),
        actions: [
          TextButton(
            onPressed: () => setState(() => _showManualEntry = !_showManualEntry),
            child: Text(_showManualEntry ? 'Scan QR' : 'Enter Code'),
          ),
        ],
      ),
      body: Consumer<PairingService>(
        builder: (_, svc, __) {
          if (svc.state == PairingState.connected) {
            return _buildSuccess(svc);
          }
          if (svc.state == PairingState.failed) {
            return _buildError(svc);
          }
          if (svc.state == PairingState.connecting) {
            return _buildConnecting();
          }
          return _showManualEntry ? _buildManualEntry(svc) : _buildQrScanner(svc);
        },
      ),
    );
  }

  Widget _buildQrScanner(PairingService svc) {
    if (!svc.hasCameraPermission) {
      return Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(Icons.camera_alt, size: 64, color: Colors.grey.shade600),
            const SizedBox(height: 16),
            Text(
              'Camera permission required',
              style: TextStyle(fontSize: 16, color: Colors.grey.shade400),
            ),
            const SizedBox(height: 8),
            Text(
              'Allow Conduit to access your camera\nto scan QR codes',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 13, color: Colors.grey.shade600),
            ),
            const SizedBox(height: 24),
            ElevatedButton(
              onPressed: () => svc.requestCameraPermission(),
              child: const Text('Grant Permission'),
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
              border: Border.all(color: const Color(0xFF6366F1), width: 2),
              borderRadius: BorderRadius.circular(16),
            ),
            child: const Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon(Icons.qr_code_scanner, size: 64, color: Color(0xFF6366F1)),
                  SizedBox(height: 16),
                  Text(
                    'Point camera at QR code',
                    style: TextStyle(color: Color(0xFFA0A0B0)),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: 24),
          Text(
            'Open Conduit on your desktop and\nscan the pairing QR code',
            textAlign: TextAlign.center,
            style: TextStyle(fontSize: 13, color: Colors.grey.shade500),
          ),
        ],
      ),
    );
  }

  Widget _buildManualEntry(PairingService svc) {
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.keyboard, size: 64, color: Colors.grey.shade600),
          const SizedBox(height: 16),
          Text(
            'Enter pairing code',
            style: TextStyle(fontSize: 16, color: Colors.grey.shade400),
          ),
          const SizedBox(height: 8),
          Text(
            'Type the code shown on your desktop',
            style: TextStyle(fontSize: 13, color: Colors.grey.shade600),
          ),
          const SizedBox(height: 24),
          TextField(
            controller: _codeController,
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 20, letterSpacing: 4),
            decoration: InputDecoration(
              hintText: 'XXXX-XXXX-XXXX',
              hintStyle: TextStyle(color: Colors.grey.shade600),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(12),
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
                  svc.pairFromCode(code, wsService);
                }
              },
              child: const Text('Connect'),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildConnecting() {
    return const Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          CircularProgressIndicator(color: Color(0xFF6366F1)),
          SizedBox(height: 24),
          Text('Connecting...', style: TextStyle(fontSize: 16)),
          SizedBox(height: 8),
          Text(
            'Establishing encrypted connection',
            style: TextStyle(fontSize: 13, color: Color(0xFFA0A0B0)),
          ),
        ],
      ),
    );
  }

  Widget _buildSuccess(PairingService svc) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Container(
            width: 80,
            height: 80,
            decoration: const BoxDecoration(
              color: Color(0xFF2DD4BF),
              shape: BoxShape.circle,
            ),
            child: const Icon(Icons.check, color: Colors.white, size: 40),
          ),
          const SizedBox(height: 24),
          const Text(
            'Paired!',
            style: TextStyle(fontSize: 24, fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 8),
          Text(
            'Connected to ${svc.pairedDevice?.deviceName ?? "desktop"}',
            style: const TextStyle(fontSize: 14, color: Color(0xFFA0A0B0)),
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

  Widget _buildError(PairingService svc) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.error_outline, size: 64, color: Color(0xFFEF4444)),
          const SizedBox(height: 16),
          Text(
            svc.error ?? 'Something went wrong',
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 14),
          ),
          const SizedBox(height: 24),
          ElevatedButton(
            onPressed: () => svc.reset(),
            child: const Text('Try Again'),
          ),
        ],
      ),
    );
  }
}
