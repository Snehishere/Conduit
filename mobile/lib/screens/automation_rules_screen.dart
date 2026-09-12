import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/automation_rule.dart';
import '../services/automation_service.dart';

class AutomationRulesScreen extends StatefulWidget {
  const AutomationRulesScreen({super.key});

  @override
  State<AutomationRulesScreen> createState() => _AutomationRulesScreenState();
}

class _AutomationRulesScreenState extends State<AutomationRulesScreen> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<AutomationService>().loadRules();
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: const Color(0xFF0A0A0F),
      appBar: AppBar(
        backgroundColor: const Color(0xFF12121A),
        elevation: 0,
        title: Row(
          children: [
            Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                color: const Color(0xFF6366F1).withValues(alpha: 0.15),
                borderRadius: BorderRadius.circular(10),
                border: Border.all(
                  color: const Color(0xFF6366F1).withValues(alpha: 0.3),
                ),
              ),
              child: const Text('⚡', style: TextStyle(fontSize: 18)),
            ),
            const SizedBox(width: 12),
            const Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Automation',
                  style: TextStyle(
                    fontSize: 18,
                    fontWeight: FontWeight.bold,
                    color: Colors.white,
                  ),
                ),
                Text(
                  'Trigger → Action Rules',
                  style: TextStyle(fontSize: 11, color: Color(0xFF606070)),
                ),
              ],
            ),
          ],
        ),
      ),
      body: Consumer<AutomationService>(
        builder: (_, service, __) {
          if (service.rules.isEmpty) return _buildEmptyState();
          return _buildRuleList(service);
        },
      ),
      floatingActionButton: FloatingActionButton(
        onPressed: () => _showCreateRuleSheet(context),
        backgroundColor: const Color(0xFF6366F1),
        elevation: 4,
        child: const Icon(Icons.add, color: Colors.white, size: 28),
      ),
    );
  }

  Widget _buildEmptyState() {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 80,
            height: 80,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: const Color(0xFF6366F1).withValues(alpha: 0.1),
              border: Border.all(
                color: const Color(0xFF6366F1).withValues(alpha: 0.2),
                width: 2,
              ),
            ),
            child: const Text('🕸️', style: TextStyle(fontSize: 36)),
          ),
          const SizedBox(height: 20),
          const Text(
            'No Automation Rules',
            style: TextStyle(
              fontSize: 18,
              fontWeight: FontWeight.w600,
              color: Colors.white,
            ),
          ),
          const SizedBox(height: 8),
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 40),
            child: Text(
              'Create rules to automate actions when triggers fire.\n'
              'Like a spider\'s web — each thread connects a cause to an effect.',
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 13,
                color: Color(0xFF606070),
                height: 1.5,
              ),
            ),
          ),
          const SizedBox(height: 24),
          GestureDetector(
            onTap: () => _showCreateRuleSheet(context),
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
              decoration: BoxDecoration(
                color: const Color(0xFF6366F1),
                borderRadius: BorderRadius.circular(12),
                boxShadow: [
                  BoxShadow(
                    color: const Color(0xFF6366F1).withValues(alpha: 0.3),
                    blurRadius: 12,
                    offset: const Offset(0, 4),
                  ),
                ],
              ),
              child: const Text(
                'Create First Rule',
                style: TextStyle(
                  color: Colors.white,
                  fontWeight: FontWeight.w600,
                  fontSize: 14,
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildRuleList(AutomationService service) {
    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 100),
      children: [
        // Stats bar
        Container(
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: const Color(0xFF12121A),
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: const Color(0xFF1E1E2E)),
          ),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceAround,
            children: [
              _buildStatItem('🕸️', '${service.rules.length}', 'Rules',
                  const Color(0xFF6366F1)),
              _buildStatItem('⚡', '${service.activeRules.length}', 'Active',
                  const Color(0xFF2DD4BF)),
              _buildStatItem(
                '💤',
                '${service.rules.length - service.activeRules.length}',
                'Paused',
                const Color(0xFFEAB308),
              ),
            ],
          ),
        ),
        const SizedBox(height: 16),
        ...service.rules.map((rule) => _buildRuleCard(rule, service)),
        const SizedBox(height: 12),
        Center(
          child: Opacity(
            opacity: 0.15,
            child: CustomPaint(
              size: const Size(200, 40),
              painter: _WebThreadPainter(),
            ),
          ),
        ),
      ],
    );
  }

  Widget _buildStatItem(String icon, String value, String label, Color color) {
    return Column(
      children: [
        Text(icon, style: const TextStyle(fontSize: 16)),
        const SizedBox(height: 4),
        Text(
          value,
          style: TextStyle(
            fontSize: 20,
            fontWeight: FontWeight.bold,
            color: color,
          ),
        ),
        Text(
          label,
          style: const TextStyle(fontSize: 11, color: Color(0xFF606070)),
        ),
      ],
    );
  }

  Widget _buildRuleCard(AutomationRule rule, AutomationService service) {
    return GestureDetector(
      onLongPress: () => _showDeleteConfirmation(rule, service),
      child: Container(
        margin: const EdgeInsets.only(bottom: 12),
        decoration: BoxDecoration(
          color: const Color(0xFF12121A),
          borderRadius: BorderRadius.circular(14),
          border: Border.all(
            color: rule.enabled
                ? const Color(0xFF6366F1).withValues(alpha: 0.3)
                : const Color(0xFF1E1E2E),
            width: rule.enabled ? 1.5 : 1,
          ),
          boxShadow: rule.enabled
              ? [
                  BoxShadow(
                    color: const Color(0xFF6366F1).withValues(alpha: 0.08),
                    blurRadius: 12,
                    offset: const Offset(0, 2),
                  ),
                ]
              : null,
        ),
        child: Column(
          children: [
            // Header
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 14, 8, 10),
              child: Row(
                children: [
                  Container(
                    width: 4,
                    height: 40,
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(2),
                      color: rule.enabled
                          ? const Color(0xFF6366F1)
                          : const Color(0xFF303040),
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          rule.name,
                          style: TextStyle(
                            fontSize: 15,
                            fontWeight: FontWeight.w600,
                            color: rule.enabled
                                ? Colors.white
                                : Colors.white.withValues(alpha: 0.5),
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          rule.id,
                          style: const TextStyle(
                            fontSize: 11,
                            color: Color(0xFF505060),
                          ),
                        ),
                      ],
                    ),
                  ),
                  Switch(
                    value: rule.enabled,
                    onChanged: (_) => service.toggleRule(rule.id),
                    activeThumbColor: const Color(0xFF6366F1),
                    activeTrackColor:
                        const Color(0xFF6366F1).withValues(alpha: 0.3),
                    inactiveThumbColor: const Color(0xFF404050),
                    inactiveTrackColor: const Color(0xFF202030),
                  ),
                ],
              ),
            ),

            // Trigger → Action visualization
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 14),
              child: Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: const Color(0xFF0A0A0F),
                  borderRadius: BorderRadius.circular(10),
                  border: Border.all(color: const Color(0xFF1E1E2E)),
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: _buildFlowPill(
                        rule.trigger.type.icon,
                        rule.trigger.type.label,
                        const Color(0xFFEAB308),
                      ),
                    ),
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      child: CustomPaint(
                        size: const Size(32, 20),
                        painter: _ThreadConnectorPainter(
                          color: rule.enabled
                              ? const Color(0xFF6366F1)
                              : const Color(0xFF303040),
                        ),
                      ),
                    ),
                    Expanded(
                      child: _buildFlowPill(
                        rule.action.type.icon,
                        rule.action.type.label,
                        const Color(0xFF2DD4BF),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildFlowPill(String icon, String label, Color accentColor) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
      decoration: BoxDecoration(
        color: accentColor.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: accentColor.withValues(alpha: 0.2)),
      ),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(icon, style: const TextStyle(fontSize: 14)),
          const SizedBox(width: 6),
          Flexible(
            child: Text(
              label,
              style: TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.w500,
                color: accentColor,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }

  void _showDeleteConfirmation(AutomationRule rule, AutomationService service) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: const Color(0xFF12121A),
        title: const Text('Remove Rule', style: TextStyle(color: Colors.white)),
        content: Text(
          'Delete "${rule.name}"?',
          style: const TextStyle(color: Color(0xFFA0A0B0)),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel',
                style: TextStyle(color: Color(0xFF606070))),
          ),
          TextButton(
            onPressed: () {
              service.removeRule(rule.id);
              Navigator.pop(ctx);
            },
            child: const Text('Delete',
                style: TextStyle(color: Color(0xFFEF4444))),
          ),
        ],
      ),
    );
  }

  void _showCreateRuleSheet(BuildContext context) {
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: const Color(0xFF12121A),
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(20)),
      ),
      builder: (_) => const _CreateRuleSheet(),
    );
  }
}

// ---------------------------------------------------------------------------
// Create Rule Bottom Sheet
// ---------------------------------------------------------------------------

class _CreateRuleSheet extends StatefulWidget {
  const _CreateRuleSheet();
  @override
  State<_CreateRuleSheet> createState() => _CreateRuleSheetState();
}

class _CreateRuleSheetState extends State<_CreateRuleSheet> {
  final _nameController = TextEditingController();
  TriggerType _selectedTrigger = TriggerType.deviceConnect;
  ActionType _selectedAction = ActionType.sendNotification;
  int _step = 0;

  // Trigger param fields
  final _triggerDeviceController = TextEditingController();
  final _triggerTimeController = TextEditingController();
  final _triggerBelowController = TextEditingController();
  final _triggerSsidController = TextEditingController();
  final _triggerAppPackageController = TextEditingController();

  // Action param fields
  final _actionTitleController = TextEditingController();
  final _actionBodyController = TextEditingController();
  final _actionProfileController = TextEditingController(text: 'silent');
  final _actionDeviceIdController = TextEditingController();
  final _actionCommandController = TextEditingController();
  final _actionUrlController = TextEditingController();
  final _actionAppPackageController = TextEditingController();
  final _actionWindowStateController = TextEditingController(text: 'minimize');

  @override
  void dispose() {
    _nameController.dispose();
    _triggerDeviceController.dispose();
    _triggerTimeController.dispose();
    _triggerBelowController.dispose();
    _triggerSsidController.dispose();
    _triggerAppPackageController.dispose();
    _actionTitleController.dispose();
    _actionBodyController.dispose();
    _actionProfileController.dispose();
    _actionDeviceIdController.dispose();
    _actionCommandController.dispose();
    _actionUrlController.dispose();
    _actionAppPackageController.dispose();
    _actionWindowStateController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return DraggableScrollableSheet(
      initialChildSize: 0.9,
      minChildSize: 0.5,
      maxChildSize: 0.95,
      expand: false,
      builder: (_, controller) => Column(
        children: [
          // Handle
          Container(
            width: 40,
            height: 4,
            margin: const EdgeInsets.only(top: 12),
            decoration: BoxDecoration(
              color: const Color(0xFF303040),
              borderRadius: BorderRadius.circular(2),
            ),
          ),

          // Header
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 16, 20, 0),
            child: Row(
              children: [
                const Text('🕸️', style: TextStyle(fontSize: 22)),
                const SizedBox(width: 10),
                const Expanded(
                  child: Text(
                    'New Automation Rule',
                    style: TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.bold,
                      color: Colors.white,
                    ),
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.close, color: Color(0xFF606070)),
                  onPressed: () => Navigator.pop(context),
                ),
              ],
            ),
          ),

          // Step indicator
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 16),
            child: Row(
              children: [
                _buildStepDot(0, 'Trigger'),
                _buildStepLine(0),
                _buildStepDot(1, 'Action'),
                _buildStepLine(1),
                _buildStepDot(2, 'Name'),
              ],
            ),
          ),

          // Content
          Expanded(
            child: ListView(
              controller: controller,
              padding: const EdgeInsets.symmetric(horizontal: 20),
              children: [
                if (_step == 0) _buildTriggerPicker(),
                if (_step == 1) _buildActionPicker(),
                if (_step == 2) _buildNameInput(),
              ],
            ),
          ),

          // Buttons
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 12, 20, 20),
            child: Row(
              children: [
                if (_step > 0)
                  Expanded(
                    child: TextButton(
                      onPressed: () => setState(() => _step--),
                      child: const Text('Back',
                          style: TextStyle(color: Color(0xFF606070))),
                    ),
                  ),
                if (_step > 0) const SizedBox(width: 12),
                Expanded(
                  flex: 2,
                  child: ElevatedButton(
                    onPressed: _step < 2
                        ? () => setState(() => _step++)
                        : _createRule,
                    style: ElevatedButton.styleFrom(
                      backgroundColor: const Color(0xFF6366F1),
                      padding: const EdgeInsets.symmetric(vertical: 14),
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(12),
                      ),
                      elevation: 0,
                    ),
                    child: Text(
                      _step < 2 ? 'Next' : '🕸️ Create Rule',
                      style: const TextStyle(
                        color: Colors.white,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildStepDot(int step, String label) {
    final active = _step >= step;
    final current = _step == step;
    return Column(
      children: [
        Container(
          width: 28,
          height: 28,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: active ? const Color(0xFF6366F1) : const Color(0xFF1E1E2E),
            border: Border.all(
              color: current
                  ? const Color(0xFF818CF8)
                  : active
                      ? const Color(0xFF6366F1)
                      : const Color(0xFF303040),
              width: current ? 2 : 1,
            ),
          ),
          child: Center(
            child: active
                ? const Icon(Icons.check, size: 14, color: Colors.white)
                : Text('${step + 1}',
                    style:
                        const TextStyle(fontSize: 11, color: Color(0xFF606070))),
          ),
        ),
        const SizedBox(height: 4),
        Text(
          label,
          style: TextStyle(
            fontSize: 10,
            color: active ? const Color(0xFF818CF8) : const Color(0xFF505060),
            fontWeight: current ? FontWeight.w600 : FontWeight.normal,
          ),
        ),
      ],
    );
  }

  Widget _buildStepLine(int afterStep) {
    return Expanded(
      child: Container(
        height: 2,
        margin: const EdgeInsets.symmetric(horizontal: 6),
        decoration: BoxDecoration(
          color:
              _step > afterStep ? const Color(0xFF6366F1) : const Color(0xFF1E1E2E),
          borderRadius: BorderRadius.circular(1),
        ),
      ),
    );
  }

  // ---- Step 0: Trigger picker ----

  Widget _buildTriggerPicker() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Text('When this happens...',
            style: TextStyle(
                fontSize: 14, fontWeight: FontWeight.w600, color: Colors.white)),
        const SizedBox(height: 4),
        const Text('Select the event that fires this rule',
            style: TextStyle(fontSize: 12, color: Color(0xFF606070))),
        const SizedBox(height: 16),
        ...TriggerType.values.map((t) => _buildTriggerOption(t)),
        const SizedBox(height: 16),
        _buildTriggerFields(),
      ],
    );
  }

  Widget _buildTriggerOption(TriggerType type) {
    final selected = _selectedTrigger == type;
    return GestureDetector(
      onTap: () => setState(() => _selectedTrigger = type),
      child: Container(
        margin: const EdgeInsets.only(bottom: 8),
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: selected
              ? const Color(0xFFEAB308).withValues(alpha: 0.08)
              : const Color(0xFF0A0A0F),
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: selected
                ? const Color(0xFFEAB308).withValues(alpha: 0.4)
                : const Color(0xFF1E1E2E),
            width: selected ? 1.5 : 1,
          ),
        ),
        child: Row(
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: const Color(0xFFEAB308).withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
              ),
              child: Center(
                child: Text(type.icon, style: const TextStyle(fontSize: 18)),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                type.label,
                style: TextStyle(
                  fontSize: 14,
                  fontWeight: FontWeight.w500,
                  color: selected
                      ? Colors.white
                      : Colors.white.withValues(alpha: 0.7),
                ),
              ),
            ),
            if (selected)
              const Icon(Icons.check_circle,
                  color: Color(0xFFEAB308), size: 20),
          ],
        ),
      ),
    );
  }

  Widget _buildTriggerFields() {
    switch (_selectedTrigger) {
      case TriggerType.deviceConnect:
      case TriggerType.deviceDisconnect:
        return _field('Device ID', _triggerDeviceController, 'e.g. d_002');
      case TriggerType.time:
        return _field('Time (HH:MM)', _triggerTimeController, 'e.g. 22:00');
      case TriggerType.batteryLevel:
        return _field('Below %', _triggerBelowController, 'e.g. 20');
      case TriggerType.wifiChange:
        return _field('SSID', _triggerSsidController, 'e.g. HomeNet');
      case TriggerType.appOpen:
        return Column(
          children: [
            _field('App Package', _triggerAppPackageController,
                'e.g. com.spotify.music'),
            const SizedBox(height: 8),
            _field('Device ID (optional)', _triggerDeviceController,
                'd_002 or * for any'),
          ],
        );
      case TriggerType.audioDeviceConnect:
      case TriggerType.audioDeviceDisconnect:
        return _field('Audio Device ID', _triggerDeviceController, 'e.g. d_003');
    }
  }

  // ---- Step 1: Action picker ----

  Widget _buildActionPicker() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // Trigger summary
        Container(
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: const Color(0xFFEAB308).withValues(alpha: 0.06),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: const Color(0xFFEAB308).withValues(alpha: 0.2),
            ),
          ),
          child: Row(
            children: [
              const Text('⚡', style: TextStyle(fontSize: 16)),
              const SizedBox(width: 8),
              const Text('Trigger: ',
                  style: TextStyle(fontSize: 12, color: Color(0xFFEAB308))),
              Text(
                _selectedTrigger.label,
                style: const TextStyle(
                    fontSize: 13,
                    fontWeight: FontWeight.w600,
                    color: Colors.white),
              ),
            ],
          ),
        ),
        const SizedBox(height: 20),

        const Text('Then do this...',
            style: TextStyle(
                fontSize: 14, fontWeight: FontWeight.w600, color: Colors.white)),
        const SizedBox(height: 4),
        const Text('Select the automated action',
            style: TextStyle(fontSize: 12, color: Color(0xFF606070))),
        const SizedBox(height: 16),
        ...ActionType.values.map((a) => _buildActionOption(a)),
        const SizedBox(height: 16),
        _buildActionFields(),
      ],
    );
  }

  Widget _buildActionOption(ActionType type) {
    final selected = _selectedAction == type;
    return GestureDetector(
      onTap: () => setState(() => _selectedAction = type),
      child: Container(
        margin: const EdgeInsets.only(bottom: 8),
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: selected
              ? const Color(0xFF2DD4BF).withValues(alpha: 0.08)
              : const Color(0xFF0A0A0F),
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: selected
                ? const Color(0xFF2DD4BF).withValues(alpha: 0.4)
                : const Color(0xFF1E1E2E),
            width: selected ? 1.5 : 1,
          ),
        ),
        child: Row(
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: const Color(0xFF2DD4BF).withValues(alpha: 0.1),
                borderRadius: BorderRadius.circular(8),
              ),
              child: Center(
                child: Text(type.icon, style: const TextStyle(fontSize: 18)),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                type.label,
                style: TextStyle(
                  fontSize: 14,
                  fontWeight: FontWeight.w500,
                  color: selected
                      ? Colors.white
                      : Colors.white.withValues(alpha: 0.7),
                ),
              ),
            ),
            if (selected)
              const Icon(Icons.check_circle,
                  color: Color(0xFF2DD4BF), size: 20),
          ],
        ),
      ),
    );
  }

  Widget _buildActionFields() {
    switch (_selectedAction) {
      case ActionType.sendNotification:
        return Column(
          children: [
            _field('Title', _actionTitleController, 'e.g. Low battery'),
            const SizedBox(height: 8),
            _field('Body', _actionBodyController, 'e.g. Phone below 20%'),
          ],
        );
      case ActionType.setPhoneProfile:
        return _dropdown('Profile', _actionProfileController,
            ['silent', 'vibrate', 'ring']);
      case ActionType.routeAudio:
        return _field('Device ID', _actionDeviceIdController, 'e.g. d_003');
      case ActionType.runShellCommand:
        return _field('Command', _actionCommandController, 'e.g. echo hello');
      case ActionType.toggleWifi:
        return _toggleField('Enable WiFi', _wifiEnabled);
      case ActionType.toggleBluetooth:
        return _toggleField('Enable Bluetooth', _btEnabled);
      case ActionType.openUrl:
        return _field('URL', _actionUrlController, 'https://...');
      case ActionType.openApp:
        return _field('App Package', _actionAppPackageController,
            'e.g. com.whatsapp');
      case ActionType.setWindowState:
        return _dropdown('Window State', _actionWindowStateController,
            ['minimize', 'maximize', 'restore', 'close']);
    }
  }

  bool _wifiEnabled = true;
  bool _btEnabled = true;

  Widget _toggleField(String label, bool value) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.spaceBetween,
      children: [
        Text(label,
            style: const TextStyle(fontSize: 14, color: Colors.white)),
        Switch(
          value: value,
          onChanged: (v) => setState(() {
            if (label.contains('WiFi')) {
              _wifiEnabled = v;
            } else {
              _btEnabled = v;
            }
          }),
          activeThumbColor: const Color(0xFF2DD4BF),
          activeTrackColor: const Color(0xFF2DD4BF).withValues(alpha: 0.3),
          inactiveThumbColor: const Color(0xFF404050),
          inactiveTrackColor: const Color(0xFF202030),
        ),
      ],
    );
  }

  // ---- Step 2: Name ----

  Widget _buildNameInput() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // Flow summary
        Container(
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: const Color(0xFF0A0A0F),
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: const Color(0xFF1E1E2E)),
          ),
          child: Row(
            children: [
              Text(_selectedTrigger.icon, style: const TextStyle(fontSize: 20)),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(_selectedTrigger.label,
                        style: const TextStyle(
                            fontSize: 13,
                            fontWeight: FontWeight.w500,
                            color: Color(0xFFEAB308))),
                    const Text('triggers',
                        style:
                            TextStyle(fontSize: 10, color: Color(0xFF505060))),
                  ],
                ),
              ),
              Container(
                padding:
                    const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                decoration: BoxDecoration(
                  color: const Color(0xFF6366F1).withValues(alpha: 0.1),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: const Text('→',
                    style: TextStyle(
                        color: Color(0xFF6366F1),
                        fontWeight: FontWeight.bold)),
              ),
              const SizedBox(width: 8),
              Text(_selectedAction.icon, style: const TextStyle(fontSize: 20)),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(_selectedAction.label,
                        style: const TextStyle(
                            fontSize: 13,
                            fontWeight: FontWeight.w500,
                            color: Color(0xFF2DD4BF))),
                    const Text('action',
                        style:
                            TextStyle(fontSize: 10, color: Color(0xFF505060))),
                  ],
                ),
              ),
            ],
          ),
        ),
        const SizedBox(height: 24),
        const Text('Name your rule',
            style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: Colors.white)),
        const SizedBox(height: 4),
        const Text('Give it a descriptive name',
            style: TextStyle(fontSize: 12, color: Color(0xFF606070))),
        const SizedBox(height: 12),
        TextField(
          controller: _nameController,
          style: const TextStyle(color: Colors.white),
          decoration: InputDecoration(
            hintText: 'e.g. Auto-notify on disconnect',
            hintStyle: const TextStyle(color: Color(0xFF404050)),
            filled: true,
            fillColor: const Color(0xFF0A0A0F),
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(color: Color(0xFF1E1E2E)),
            ),
            enabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(color: Color(0xFF1E1E2E)),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(12),
              borderSide: const BorderSide(
                  color: Color(0xFF6366F1), width: 1.5),
            ),
            contentPadding: const EdgeInsets.all(14),
          ),
          onChanged: (_) => setState(() {}),
        ),
      ],
    );
  }

  // ---- Helpers ----

  Widget _field(
      String label, TextEditingController ctrl, String hint) {
    return TextField(
      controller: ctrl,
      style: const TextStyle(color: Colors.white, fontSize: 14),
      decoration: InputDecoration(
        labelText: label,
        labelStyle: const TextStyle(color: Color(0xFF808090)),
        hintText: hint,
        hintStyle: const TextStyle(color: Color(0xFF404050)),
        filled: true,
        fillColor: const Color(0xFF0A0A0F),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: const BorderSide(color: Color(0xFF1E1E2E)),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: const BorderSide(color: Color(0xFF1E1E2E)),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide:
              const BorderSide(color: Color(0xFF6366F1), width: 1.5),
        ),
        contentPadding: const EdgeInsets.all(12),
      ),
    );
  }

  Widget _dropdown(
      String label, TextEditingController ctrl, List<String> options) {
    return DropdownButtonFormField<String>(
      initialValue: ctrl.text.isEmpty ? null : ctrl.text,
      dropdownColor: const Color(0xFF12121A),
      style: const TextStyle(color: Colors.white, fontSize: 14),
      decoration: InputDecoration(
        labelText: label,
        labelStyle: const TextStyle(color: Color(0xFF808090)),
        filled: true,
        fillColor: const Color(0xFF0A0A0F),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: const BorderSide(color: Color(0xFF1E1E2E)),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: const BorderSide(color: Color(0xFF1E1E2E)),
        ),
      ),
      items: options
          .map((o) => DropdownMenuItem(value: o, child: Text(o)))
          .toList(),
      onChanged: (v) {
        if (v != null) ctrl.text = v;
      },
    );
  }

  void _createRule() {
    final name = _nameController.text.trim();
    if (name.isEmpty) return;

    final trigger = _buildTriggerFromFields();
    final action = _buildActionFromFields();

    final rule = AutomationRule(
      id: 'rule_${DateTime.now().millisecondsSinceEpoch}',
      name: name,
      trigger: trigger,
      action: action,
      enabled: true,
    );

    context.read<AutomationService>().addRule(rule);
    Navigator.pop(context);
  }

  Trigger _buildTriggerFromFields() {
    switch (_selectedTrigger) {
      case TriggerType.deviceConnect:
      case TriggerType.deviceDisconnect:
        return Trigger(
            type: _selectedTrigger,
            deviceId: _triggerDeviceController.text);
      case TriggerType.time:
        return Trigger(type: _selectedTrigger, time: _triggerTimeController.text);
      case TriggerType.batteryLevel:
        return Trigger(
            type: _selectedTrigger,
            below: int.tryParse(_triggerBelowController.text));
      case TriggerType.wifiChange:
        return Trigger(
            type: _selectedTrigger, ssid: _triggerSsidController.text);
      case TriggerType.appOpen:
        return Trigger(
          type: _selectedTrigger,
          appPackage: _triggerAppPackageController.text,
          deviceId: _triggerDeviceController.text,
        );
      case TriggerType.audioDeviceConnect:
      case TriggerType.audioDeviceDisconnect:
        return Trigger(
            type: _selectedTrigger,
            deviceId: _triggerDeviceController.text);
    }
  }

  AutomationAction _buildActionFromFields() {
    switch (_selectedAction) {
      case ActionType.sendNotification:
        return AutomationAction(
          type: _selectedAction,
          title: _actionTitleController.text,
          body: _actionBodyController.text,
        );
      case ActionType.setPhoneProfile:
        return AutomationAction(
            type: _selectedAction, profile: _actionProfileController.text);
      case ActionType.routeAudio:
        return AutomationAction(
            type: _selectedAction, deviceId: _actionDeviceIdController.text);
      case ActionType.runShellCommand:
        return AutomationAction(
            type: _selectedAction, command: _actionCommandController.text);
      case ActionType.toggleWifi:
        return AutomationAction(
            type: _selectedAction, enabled: _wifiEnabled);
      case ActionType.toggleBluetooth:
        return AutomationAction(
            type: _selectedAction, enabled: _btEnabled);
      case ActionType.openUrl:
        return AutomationAction(
            type: _selectedAction, url: _actionUrlController.text);
      case ActionType.openApp:
        return AutomationAction(
            type: _selectedAction,
            appPackage: _actionAppPackageController.text);
      case ActionType.setWindowState:
        return AutomationAction(
            type: _selectedAction,
            state: _actionWindowStateController.text);
    }
  }
}

// ---------------------------------------------------------------------------
// Custom Painters
// ---------------------------------------------------------------------------

class _ThreadConnectorPainter extends CustomPainter {
  final Color color;
  _ThreadConnectorPainter({required this.color});

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = 2
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round;

    const dashWidth = 4.0;
    const dashSpace = 3.0;
    double start = 0;
    while (start < size.width) {
      final end = (start + dashWidth).clamp(0.0, size.width);
      canvas.drawLine(
        Offset(start, size.height / 2),
        Offset(end, size.height / 2),
        paint,
      );
      start += dashWidth + dashSpace;
    }

    final arrowPaint = Paint()
      ..color = color
      ..style = PaintingStyle.fill;
    final path = Path()
      ..moveTo(size.width - 4, size.height / 2 - 4)
      ..lineTo(size.width, size.height / 2)
      ..lineTo(size.width - 4, size.height / 2 + 4)
      ..close();
    canvas.drawPath(path, arrowPaint);
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

class _WebThreadPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = const Color(0xFF6366F1)
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke;

    final center = Offset(size.width / 2, size.height / 2);
    final radius = size.height / 2;

    for (double r = 8; r <= radius; r += 8) {
      final rect = Rect.fromCircle(center: center, radius: r);
      canvas.drawArc(rect, 0.3, 2.5, false, paint);
    }

    for (double angle = 0; angle < 6.28; angle += 0.78) {
      final end = Offset(
        center.dx + radius * 1.1 * angle / 6.28 * 2 - radius * 0.55,
        center.dy,
      );
      canvas.drawLine(center, end, paint);
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}
