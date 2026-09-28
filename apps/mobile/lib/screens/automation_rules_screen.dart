import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/automation_rule.dart';
import '../services/automation_service.dart';
import '../theme/app_theme.dart';
import '../widgets/empty_state.dart';

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
    final colors = Theme.of(context).extension<AppColors>()!;

    return Scaffold(
      appBar: AppBar(
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Automation',
              style: TextStyle(
                fontSize: 18,
                fontWeight: FontWeight.bold,
                color: colors.text1,
              ),
            ),
            Text(
              'Trigger and action rules',
              style: TextStyle(fontSize: 11, color: colors.text3),
            ),
          ],
        ),
      ),
      body: Consumer<AutomationService>(
        builder: (_, service, _) {
          if (service.rules.isEmpty) return _buildEmptyState(colors);
          return _buildRuleList(service, colors);
        },
      ),
      floatingActionButton: FloatingActionButton(
        onPressed: () => _showCreateRuleSheet(context),
        backgroundColor: colors.accentSecondary,
        elevation: 4,
        child: Icon(Icons.add, color: colors.bg0, size: 28),
      ),
    );
  }

  Widget _buildEmptyState(AppColors colors) {
    return EmptyState(
      icon: Icons.hub_outlined,
      title: 'No automation rules',
      description:
          'A rule runs an action on this device when its '
          'trigger condition is met.',
      actionLabel: 'Create rule',
      onAction: () => _showCreateRuleSheet(context),
    );
  }

  Widget _buildRuleList(AutomationService service, AppColors colors) {
    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 100),
      children: [
        // Stats bar
        Container(
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: colors.bg1,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: colors.border),
          ),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceAround,
            children: [
              _buildStatItem(
                '${service.rules.length}',
                'Rules',
                colors.accentSecondary,
                colors,
              ),
              _buildStatItem(
                '${service.activeRules.length}',
                'Active',
                colors.accent,
                colors,
              ),
              _buildStatItem(
                '${service.rules.length - service.activeRules.length}',
                'Paused',
                colors.warning,
                colors,
              ),
            ],
          ),
        ),
        const SizedBox(height: 16),
        ...service.rules.map((rule) => _buildRuleCard(rule, service, colors)),
        const SizedBox(height: 12),
        Center(
          child: Opacity(
            opacity: 0.15,
            child: CustomPaint(
              size: const Size(200, 40),
              painter: _WebThreadPainter(color: colors.accentSecondary),
            ),
          ),
        ),
      ],
    );
  }

  Widget _buildStatItem(
    String value,
    String label,
    Color color,
    AppColors colors,
  ) {
    return Column(
      children: [
        Text(
          value,
          style: TextStyle(
            fontSize: 20,
            fontWeight: FontWeight.bold,
            color: color,
          ),
        ),
        const SizedBox(height: 4),
        Text(
          label,
          style: TextStyle(fontSize: 11, color: colors.text3),
        ),
      ],
    );
  }

  Widget _buildRuleCard(AutomationRule rule, AutomationService service, AppColors colors) {
    return GestureDetector(
      onLongPress: () => _showDeleteConfirmation(rule, service, colors),
      child: Container(
        margin: const EdgeInsets.only(bottom: 12),
        decoration: BoxDecoration(
          color: colors.bg1,
          borderRadius: BorderRadius.circular(14),
          border: Border.all(
            color: rule.enabled
                ? colors.accentSecondary.withValues(alpha: 0.3)
                : colors.border,
            width: rule.enabled ? 1.5 : 1,
          ),
          boxShadow: rule.enabled
              ? [
                  BoxShadow(
                    color: colors.accentSecondary.withValues(alpha: 0.08),
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
                      color: rule.enabled ? colors.accentSecondary : colors.bg3,
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
                            color: rule.enabled ? colors.text1 : colors.text3,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          rule.id,
                          style: TextStyle(fontSize: 11, color: colors.text3),
                        ),
                      ],
                    ),
                  ),
                  Switch(
                    value: rule.enabled,
                    onChanged: (_) => service.toggleRule(rule.id),
                    activeThumbColor: colors.accentSecondary,
                    activeTrackColor: colors.accentSecondary.withValues(alpha: 0.3),
                    inactiveThumbColor: colors.bg3,
                    inactiveTrackColor: colors.bg2,
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
                  color: colors.bg0,
                  borderRadius: BorderRadius.circular(10),
                  border: Border.all(color: colors.border),
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: _buildFlowPill(
                        rule.trigger.type.icon,
                        rule.trigger.type.label,
                        colors.warning,
                        colors,
                      ),
                    ),
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      child: CustomPaint(
                        size: const Size(32, 20),
                        painter: _ThreadConnectorPainter(
                          color: rule.enabled ? colors.accentSecondary : colors.bg3,
                        ),
                      ),
                    ),
                    Expanded(
                      child: _buildFlowPill(
                        rule.action.type.icon,
                        rule.action.type.label,
                        colors.accent,
                        colors,
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

  Widget _buildFlowPill(String icon, String label, Color accentColor, AppColors colors) {
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

  void _showDeleteConfirmation(AutomationRule rule, AutomationService service, AppColors colors) {
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: colors.bg1,
        title: Text('Delete rule', style: TextStyle(color: colors.text1)),
        content: Text(
          'Delete "${rule.name}"?',
          style: TextStyle(color: colors.text2),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: Text('Cancel', style: TextStyle(color: colors.text3)),
          ),
          TextButton(
            onPressed: () {
              service.removeRule(rule.id);
              Navigator.pop(ctx);
            },
            child: Text('Delete', style: TextStyle(color: colors.error)),
          ),
        ],
      ),
    );
  }

  void _showCreateRuleSheet(BuildContext context) {
    final colors = Theme.of(context).extension<AppColors>()!;
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: colors.bg1,
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

  final _triggerDeviceController = TextEditingController();
  final _triggerTimeController = TextEditingController();
  final _triggerBelowController = TextEditingController();
  final _triggerSsidController = TextEditingController();
  final _triggerAppPackageController = TextEditingController();

  final _actionTitleController = TextEditingController();
  final _actionBodyController = TextEditingController();
  final _actionProfileController = TextEditingController(text: 'silent');
  final _actionDeviceIdController = TextEditingController();
  final _actionCommandController = TextEditingController();
  final _actionUrlController = TextEditingController();
  final _actionAppPackageController = TextEditingController();
  final _actionWindowStateController = TextEditingController(text: 'minimize');

  bool _wifiEnabled = true;
  bool _btEnabled = true;

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
    final colors = Theme.of(context).extension<AppColors>()!;

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
              color: colors.bg3,
              borderRadius: BorderRadius.circular(2),
            ),
          ),

          // Header
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 16, 20, 0),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    'New automation rule',
                    style: TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.bold,
                      color: colors.text1,
                    ),
                  ),
                ),
                IconButton(
                  icon: Icon(Icons.close, color: colors.text3),
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
                _buildStepDot(0, 'Trigger', colors),
                _buildStepLine(0, colors),
                _buildStepDot(1, 'Action', colors),
                _buildStepLine(1, colors),
                _buildStepDot(2, 'Name', colors),
              ],
            ),
          ),

          // Content
          Expanded(
            child: ListView(
              controller: controller,
              padding: const EdgeInsets.symmetric(horizontal: 20),
              children: [
                if (_step == 0) _buildTriggerPicker(colors),
                if (_step == 1) _buildActionPicker(colors),
                if (_step == 2) _buildNameInput(colors),
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
                      child: Text('Back', style: TextStyle(color: colors.text3)),
                    ),
                  ),
                if (_step > 0) const SizedBox(width: 12),
                Expanded(
                  flex: 2,
                  child: FilledButton(
                    onPressed: _step < 2
                        ? () => setState(() => _step++)
                        : _createRule,
                    style: FilledButton.styleFrom(
                      backgroundColor: colors.accentSecondary,
                      padding: const EdgeInsets.symmetric(vertical: 14),
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(12),
                      ),
                    ),
                    child: Text(
                      _step < 2 ? 'Next' : 'Create rule',
                      style: TextStyle(
                        color: colors.bg0,
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

  Widget _buildStepDot(int step, String label, AppColors colors) {
    final active = _step >= step;
    final current = _step == step;
    return Column(
      children: [
        Container(
          width: 28,
          height: 28,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: active ? colors.accentSecondary : colors.bg2,
            border: Border.all(
              color: current
                  ? colors.accentSecondary
                  : active
                      ? colors.accentSecondary
                      : colors.border,
              width: current ? 2 : 1,
            ),
          ),
          child: Center(
            child: active
                ? Icon(Icons.check, size: 14, color: colors.bg0)
                : Text('${step + 1}', style: TextStyle(fontSize: 11, color: colors.text3)),
          ),
        ),
        const SizedBox(height: 4),
        Text(
          label,
          style: TextStyle(
            fontSize: 10,
            color: active ? colors.accentSecondary : colors.text3,
            fontWeight: current ? FontWeight.w600 : FontWeight.normal,
          ),
        ),
      ],
    );
  }

  Widget _buildStepLine(int afterStep, AppColors colors) {
    return Expanded(
      child: Container(
        height: 2,
        margin: const EdgeInsets.symmetric(horizontal: 6),
        decoration: BoxDecoration(
          color: _step > afterStep ? colors.accentSecondary : colors.border,
          borderRadius: BorderRadius.circular(1),
        ),
      ),
    );
  }

  // ---- Step 0: Trigger picker ----

  Widget _buildTriggerPicker(AppColors colors) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('When this happens...',
            style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600, color: colors.text1)),
        const SizedBox(height: 4),
        Text('Select the event that fires this rule',
            style: TextStyle(fontSize: 12, color: colors.text3)),
        const SizedBox(height: 16),
        ...TriggerType.values.map((t) => _buildTriggerOption(t, colors)),
        const SizedBox(height: 16),
        _buildTriggerFields(colors),
      ],
    );
  }

  Widget _buildTriggerOption(TriggerType type, AppColors colors) {
    final selected = _selectedTrigger == type;
    return GestureDetector(
      onTap: () => setState(() => _selectedTrigger = type),
      child: Container(
        margin: const EdgeInsets.only(bottom: 8),
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: selected ? colors.warning.withValues(alpha: 0.08) : colors.bg0,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: selected ? colors.warning.withValues(alpha: 0.4) : colors.border,
            width: selected ? 1.5 : 1,
          ),
        ),
        child: Row(
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: colors.warning.withValues(alpha: 0.1),
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
                  color: selected ? colors.text1 : colors.text2,
                ),
              ),
            ),
            if (selected)
              Icon(Icons.check_circle, color: colors.warning, size: 20),
          ],
        ),
      ),
    );
  }

  Widget _buildTriggerFields(AppColors colors) {
    switch (_selectedTrigger) {
      case TriggerType.deviceConnect:
      case TriggerType.deviceDisconnect:
        return _field('Device ID', _triggerDeviceController, 'e.g. d_002', colors);
      case TriggerType.time:
        return _field('Time (HH:MM)', _triggerTimeController, 'e.g. 22:00', colors);
      case TriggerType.batteryLevel:
        return _field('Below %', _triggerBelowController, 'e.g. 20', colors);
      case TriggerType.wifiChange:
        return _field('SSID', _triggerSsidController, 'e.g. HomeNet', colors);
      case TriggerType.appOpen:
        return Column(
          children: [
            _field('App Package', _triggerAppPackageController,
                'e.g. com.spotify.music', colors),
            const SizedBox(height: 8),
            _field('Device ID (optional)', _triggerDeviceController,
                'd_002 or * for any', colors),
          ],
        );
      case TriggerType.audioDeviceConnect:
      case TriggerType.audioDeviceDisconnect:
        return _field('Audio Device ID', _triggerDeviceController, 'e.g. d_003', colors);
    }
  }

  // ---- Step 1: Action picker ----

  Widget _buildActionPicker(AppColors colors) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Container(
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: colors.warning.withValues(alpha: 0.06),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: colors.warning.withValues(alpha: 0.2)),
          ),
          child: Row(
            children: [
              Text('Trigger: ', style: TextStyle(fontSize: 12, color: colors.warning)),
              Text(
                _selectedTrigger.label,
                style: TextStyle(fontSize: 13, fontWeight: FontWeight.w600, color: colors.text1),
              ),
            ],
          ),
        ),
        const SizedBox(height: 20),

        Text('Then do this...',
            style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600, color: colors.text1)),
        const SizedBox(height: 4),
        Text('Select the automated action',
            style: TextStyle(fontSize: 12, color: colors.text3)),
        const SizedBox(height: 16),
        ...ActionType.values.map((a) => _buildActionOption(a, colors)),
        const SizedBox(height: 16),
        _buildActionFields(colors),
      ],
    );
  }

  Widget _buildActionOption(ActionType type, AppColors colors) {
    final selected = _selectedAction == type;
    return GestureDetector(
      onTap: () => setState(() => _selectedAction = type),
      child: Container(
        margin: const EdgeInsets.only(bottom: 8),
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: selected ? colors.accent.withValues(alpha: 0.08) : colors.bg0,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: selected ? colors.accent.withValues(alpha: 0.4) : colors.border,
            width: selected ? 1.5 : 1,
          ),
        ),
        child: Row(
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: colors.accent.withValues(alpha: 0.1),
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
                  color: selected ? colors.text1 : colors.text2,
                ),
              ),
            ),
            if (selected)
              Icon(Icons.check_circle, color: colors.accent, size: 20),
          ],
        ),
      ),
    );
  }

  Widget _buildActionFields(AppColors colors) {
    switch (_selectedAction) {
      case ActionType.sendNotification:
        return Column(
          children: [
            _field('Title', _actionTitleController, 'e.g. Low battery', colors),
            const SizedBox(height: 8),
            _field('Body', _actionBodyController, 'e.g. Phone below 20%', colors),
          ],
        );
      case ActionType.setPhoneProfile:
        return _dropdown('Profile', _actionProfileController,
            ['silent', 'vibrate', 'ring'], colors);
      case ActionType.routeAudio:
        return _field('Device ID', _actionDeviceIdController, 'e.g. d_003', colors);
      case ActionType.runShellCommand:
        return _field('Command', _actionCommandController, 'e.g. echo hello', colors);
      case ActionType.toggleWifi:
        return _toggleField('Enable WiFi', _wifiEnabled, colors, (v) {
          setState(() => _wifiEnabled = v);
        });
      case ActionType.toggleBluetooth:
        return _toggleField('Enable Bluetooth', _btEnabled, colors, (v) {
          setState(() => _btEnabled = v);
        });
      case ActionType.openUrl:
        return _field('URL', _actionUrlController, 'https://...', colors);
      case ActionType.openApp:
        return _field('App Package', _actionAppPackageController,
            'e.g. com.whatsapp', colors);
      case ActionType.setWindowState:
        return _dropdown('Window State', _actionWindowStateController,
            ['minimize', 'maximize', 'restore', 'close'], colors);
    }
  }

  Widget _toggleField(String label, bool value, AppColors colors, ValueChanged<bool> onChanged) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.spaceBetween,
      children: [
        Text(label, style: TextStyle(fontSize: 14, color: colors.text1)),
        Switch(
          value: value,
          onChanged: onChanged,
          activeThumbColor: colors.accent,
          activeTrackColor: colors.accent.withValues(alpha: 0.3),
          inactiveThumbColor: colors.bg3,
          inactiveTrackColor: colors.bg2,
        ),
      ],
    );
  }

  // ---- Step 2: Name ----

  Widget _buildNameInput(AppColors colors) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Container(
          padding: const EdgeInsets.all(14),
          decoration: BoxDecoration(
            color: colors.bg0,
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: colors.border),
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
                        style: TextStyle(fontSize: 13, fontWeight: FontWeight.w500, color: colors.warning)),
                    Text('triggers', style: TextStyle(fontSize: 10, color: colors.text3)),
                  ],
                ),
              ),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                decoration: BoxDecoration(
                  color: colors.accentSecondary.withValues(alpha: 0.1),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text('→',
                    style: TextStyle(color: colors.accentSecondary, fontWeight: FontWeight.bold)),
              ),
              const SizedBox(width: 8),
              Text(_selectedAction.icon, style: const TextStyle(fontSize: 20)),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(_selectedAction.label,
                        style: TextStyle(fontSize: 13, fontWeight: FontWeight.w500, color: colors.accent)),
                    Text('action', style: TextStyle(fontSize: 10, color: colors.text3)),
                  ],
                ),
              ),
            ],
          ),
        ),
        const SizedBox(height: 24),
        Text('Name your rule',
            style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600, color: colors.text1)),
        const SizedBox(height: 4),
        Text('Give it a descriptive name',
            style: TextStyle(fontSize: 12, color: colors.text3)),
        const SizedBox(height: 12),
        TextField(
          controller: _nameController,
          style: TextStyle(color: colors.text1),
          decoration: InputDecoration(
            hintText: 'e.g. Auto-notify on disconnect',
            hintStyle: TextStyle(color: colors.text3),
            filled: true,
            fillColor: colors.bg0,
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
              borderSide: BorderSide(color: colors.accentSecondary, width: 1.5),
            ),
            contentPadding: const EdgeInsets.all(14),
          ),
          onChanged: (_) => setState(() {}),
        ),
      ],
    );
  }

  // ---- Helpers ----

  Widget _field(String label, TextEditingController ctrl, String hint, AppColors colors) {
    return TextField(
      controller: ctrl,
      style: TextStyle(color: colors.text1, fontSize: 14),
      decoration: InputDecoration(
        labelText: label,
        labelStyle: TextStyle(color: colors.text3),
        hintText: hint,
        hintStyle: TextStyle(color: colors.text3),
        filled: true,
        fillColor: colors.bg0,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.border),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.border),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.accentSecondary, width: 1.5),
        ),
        contentPadding: const EdgeInsets.all(12),
      ),
    );
  }

  Widget _dropdown(String label, TextEditingController ctrl, List<String> options, AppColors colors) {
    return DropdownButtonFormField<String>(
      // ignore: deprecated_member_use -- `value` kept for Flutter < 3.33 compatibility (CI uses 3.24).
      value: ctrl.text.isEmpty ? null : ctrl.text,
      dropdownColor: colors.bg1,
      style: TextStyle(color: colors.text1, fontSize: 14),
      decoration: InputDecoration(
        labelText: label,
        labelStyle: TextStyle(color: colors.text3),
        filled: true,
        fillColor: colors.bg0,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.border),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: colors.border),
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
        return Trigger(type: _selectedTrigger, deviceId: _triggerDeviceController.text);
      case TriggerType.time:
        return Trigger(type: _selectedTrigger, time: _triggerTimeController.text);
      case TriggerType.batteryLevel:
        return Trigger(type: _selectedTrigger, below: int.tryParse(_triggerBelowController.text));
      case TriggerType.wifiChange:
        return Trigger(type: _selectedTrigger, ssid: _triggerSsidController.text);
      case TriggerType.appOpen:
        return Trigger(
          type: _selectedTrigger,
          appPackage: _triggerAppPackageController.text,
          deviceId: _triggerDeviceController.text,
        );
      case TriggerType.audioDeviceConnect:
      case TriggerType.audioDeviceDisconnect:
        return Trigger(type: _selectedTrigger, deviceId: _triggerDeviceController.text);
    }
  }

  AutomationAction _buildActionFromFields() {
    switch (_selectedAction) {
      case ActionType.sendNotification:
        return AutomationAction(type: _selectedAction, title: _actionTitleController.text, body: _actionBodyController.text);
      case ActionType.setPhoneProfile:
        return AutomationAction(type: _selectedAction, profile: _actionProfileController.text);
      case ActionType.routeAudio:
        return AutomationAction(type: _selectedAction, deviceId: _actionDeviceIdController.text);
      case ActionType.runShellCommand:
        return AutomationAction(type: _selectedAction, command: _actionCommandController.text);
      case ActionType.toggleWifi:
        return AutomationAction(type: _selectedAction, enabled: _wifiEnabled);
      case ActionType.toggleBluetooth:
        return AutomationAction(type: _selectedAction, enabled: _btEnabled);
      case ActionType.openUrl:
        return AutomationAction(type: _selectedAction, url: _actionUrlController.text);
      case ActionType.openApp:
        return AutomationAction(type: _selectedAction, appPackage: _actionAppPackageController.text);
      case ActionType.setWindowState:
        return AutomationAction(type: _selectedAction, state: _actionWindowStateController.text);
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
  final Color color;
  _WebThreadPainter({required this.color});

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
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
