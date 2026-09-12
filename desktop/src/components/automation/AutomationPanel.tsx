import React, { useState } from 'react';
import { useAutomation } from '../../hooks/useAutomation';
import { IconZap, IconPlus, IconX, IconEdit, IconTrash, IconBell, IconDevicePhone, IconVolume, IconDeviceDesktop, IconWifi, IconBluetooth, IconLink, IconFolder } from '../icons';

interface AutomationPanelProps {
  onRuleCreated?: (rule: any) => void;
  onRuleDeleted?: (ruleId: string) => void;
}

export default function AutomationPanel({ onRuleCreated, onRuleDeleted }: AutomationPanelProps) {
  const {
    rules,
    isEditing,
    editingRule,
    createRule,
    updateRuleById,
    deleteRuleById,
    toggleRule,
    startEditing,
    stopEditing,
  } = useAutomation();

  const [ruleName, setRuleName] = useState('');
  const [selectedTrigger, setSelectedTrigger] = useState<string>('device_connect');
  const [selectedAction, setSelectedAction] = useState<string>('send_notification');
  const [triggerConfig, setTriggerConfig] = useState<any>({});
  const [actionConfig, setActionConfig] = useState<any>({});

  const triggerTypes = [
    { id: 'device_connect', label: 'Device Connects', icon: IconLink },
    { id: 'device_disconnect', label: 'Device Disconnects', icon: IconLink },
    { id: 'time', label: 'Time of Day', icon: IconZap },
    { id: 'battery_level', label: 'Battery Level', icon: IconZap },
    { id: 'wifi_change', label: 'WiFi Network Change', icon: IconWifi },
    { id: 'app_open', label: 'App Opens', icon: IconDevicePhone },
    { id: 'audio_device_connect', label: 'Audio Device Connects', icon: IconVolume },
  ];

  const actionTypes = [
    { id: 'send_notification', label: 'Send Notification', icon: IconBell },
    { id: 'set_phone_profile', label: 'Set Phone Profile', icon: IconDevicePhone },
    { id: 'route_audio', label: 'Route Audio', icon: IconVolume },
    { id: 'run_shell_command', label: 'Run Command', icon: IconDeviceDesktop },
    { id: 'toggle_wifi', label: 'Toggle WiFi', icon: IconWifi },
    { id: 'toggle_bluetooth', label: 'Toggle Bluetooth', icon: IconBluetooth },
    { id: 'open_url', label: 'Open URL', icon: IconLink },
    { id: 'open_app', label: 'Open App', icon: IconFolder },
  ];

  const handleSave = () => {
    const rule = {
      name: ruleName,
      trigger: { type: selectedTrigger, ...triggerConfig },
      action: { type: selectedAction, ...actionConfig },
      enabled: true,
    };

    if (editingRule) {
      updateRuleById(editingRule.id, rule);
    } else {
      createRule(rule);
    }

    stopEditing();
    setRuleName('');
    setSelectedTrigger('device_connect');
    setSelectedAction('send_notification');
    setTriggerConfig({});
    setActionConfig({});
  };

  const handleEdit = (rule: any) => {
    setRuleName(rule.name);
    setSelectedTrigger(rule.trigger.type);
    setSelectedAction(rule.action.type);
    setTriggerConfig(rule.trigger);
    setActionConfig(rule.action);
    startEditing(rule);
  };

  return (
    <div className="automation-panel">
      <div className="automation-header">
        <h3>Automation Rules</h3>
        <button onClick={() => startEditing()} className="add-rule-btn">
          <IconPlus size={14} /> Add Rule
        </button>
      </div>

      {rules.length === 0 ? (
        <div className="no-rules">
          <div className="no-rules-icon">
            <IconZap size={64} />
          </div>
          <p>No automation rules</p>
          <p className="no-rules-hint">Create rules to automate tasks between your devices</p>
        </div>
      ) : (
        <div className="rules-list">
          {rules.map(rule => (
            <div key={rule.id} className={`rule-item ${!rule.enabled ? 'disabled' : ''}`}>
              <div className="rule-header">
                <div className="rule-info">
                  <div className="rule-name">{rule.name}</div>
                  <div className="rule-trigger">
                    {(() => { const T = triggerTypes.find(t => t.id === rule.trigger.type)?.icon; return T ? <T size={12} /> : null; })()}{' '}
                    {triggerTypes.find(t => t.id === rule.trigger.type)?.label}
                  </div>
                  <div className="rule-action">
                    {(() => { const A = actionTypes.find(a => a.id === rule.action.type)?.icon; return A ? <A size={12} /> : null; })()}{' '}
                    {actionTypes.find(a => a.id === rule.action.type)?.label}
                  </div>
                </div>
                <div className="rule-controls">
                  <button
                    onClick={() => toggleRule(rule.id)}
                    className={`toggle-btn ${rule.enabled ? 'enabled' : 'disabled'}`}
                  >
                    {rule.enabled ? 'ON' : 'OFF'}
                  </button>
                  <button onClick={() => handleEdit(rule)} className="edit-btn">
                    <IconEdit size={12} />
                  </button>
                  <button
                    onClick={() => {
                      deleteRuleById(rule.id);
                      onRuleDeleted?.(rule.id);
                    }}
                    className="delete-btn"
                  >
                    <IconTrash size={12} />
                  </button>
                </div>
              </div>
              <div className="rule-stats">
                <span>Triggered {rule.triggerCount} times</span>
                {rule.lastTriggered && (
                  <span>Last: {new Date(rule.lastTriggered * 1000).toLocaleString()}</span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {isEditing && (
        <div className="rule-editor-overlay">
          <div className="rule-editor">
            <div className="rule-editor-header">
              <h4>{editingRule ? 'Edit Rule' : 'Create Rule'}</h4>
              <button onClick={stopEditing} className="close-btn">
                <IconX size={18} />
              </button>
            </div>

            <div className="rule-editor-content">
              <div className="form-group">
                <label>Rule Name</label>
                <input
                  type="text"
                  value={ruleName}
                  onChange={(e) => setRuleName(e.target.value)}
                  placeholder="Enter rule name"
                  className="form-input"
                />
              </div>

              <div className="form-group">
                <label>When this happens:</label>
                <div className="trigger-options">
                  {triggerTypes.map(trigger => {
                    const TriggerIcon = trigger.icon;
                    return (
                      <button
                        key={trigger.id}
                        className={`trigger-option ${selectedTrigger === trigger.id ? 'selected' : ''}`}
                        onClick={() => {
                          setSelectedTrigger(trigger.id);
                          setTriggerConfig({});
                        }}
                      >
                        <span className="trigger-icon"><TriggerIcon size={16} /></span>
                        <span className="trigger-label">{trigger.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="form-group">
                <label>Configure Trigger:</label>
                {selectedTrigger === 'time' && (
                  <input
                    type="time"
                    value={triggerConfig.time || ''}
                    onChange={(e) => setTriggerConfig({ ...triggerConfig, time: e.target.value })}
                    className="form-input"
                  />
                )}
                {selectedTrigger === 'battery_level' && (
                  <div className="battery-config">
                    <input
                      type="range"
                      min="0"
                      max="100"
                      value={triggerConfig.battery_threshold || 20}
                      onChange={(e) => setTriggerConfig({
                        ...triggerConfig,
                        battery_threshold: parseInt(e.target.value),
                      })}
                      className="battery-slider"
                    />
                    <span className="battery-value">
                      {triggerConfig.battery_threshold || 20}%
                    </span>
                  </div>
                )}
                {selectedTrigger === 'wifi_change' && (
                  <input
                    type="text"
                    value={triggerConfig.wifi_ssid || ''}
                    onChange={(e) => setTriggerConfig({ ...triggerConfig, wifi_ssid: e.target.value })}
                    placeholder="WiFi SSID"
                    className="form-input"
                  />
                )}
              </div>

              <div className="form-group">
                <label>Do this:</label>
                <div className="action-options">
                  {actionTypes.map(action => {
                    const ActionIcon = action.icon;
                    return (
                      <button
                        key={action.id}
                        className={`action-option ${selectedAction === action.id ? 'selected' : ''}`}
                        onClick={() => {
                          setSelectedAction(action.id);
                          setActionConfig({});
                        }}
                      >
                        <span className="action-icon"><ActionIcon size={16} /></span>
                        <span className="action-label">{action.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="form-group">
                <label>Configure Action:</label>
                {selectedAction === 'send_notification' && (
                  <div className="notification-config">
                    <input
                      type="text"
                      value={actionConfig.title || ''}
                      onChange={(e) => setActionConfig({
                        ...actionConfig,
                        title: e.target.value,
                      })}
                      placeholder="Notification title"
                      className="form-input"
                    />
                    <input
                      type="text"
                      value={actionConfig.body || ''}
                      onChange={(e) => setActionConfig({
                        ...actionConfig,
                        body: e.target.value,
                      })}
                      placeholder="Notification body"
                      className="form-input"
                    />
                  </div>
                )}
                {selectedAction === 'set_phone_profile' && (
                  <select
                    value={actionConfig.profile || 'silent'}
                    onChange={(e) => setActionConfig({ ...actionConfig, profile: e.target.value })}
                    className="form-select"
                  >
                    <option value="silent">Silent</option>
                    <option value="vibrate">Vibrate</option>
                    <option value="ring">Ring</option>
                  </select>
                )}
                {selectedAction === 'run_shell_command' && (
                  <input
                    type="text"
                    value={actionConfig.command || ''}
                    onChange={(e) => setActionConfig({ ...actionConfig, command: e.target.value })}
                    placeholder="Command to run"
                    className="form-input"
                  />
                )}
                {selectedAction === 'open_url' && (
                  <input
                    type="url"
                    value={actionConfig.url || ''}
                    onChange={(e) => setActionConfig({ ...actionConfig, url: e.target.value })}
                    placeholder="URL to open"
                    className="form-input"
                  />
                )}
              </div>
            </div>

            <div className="rule-editor-footer">
              <button onClick={stopEditing} className="cancel-btn">
                Cancel
              </button>
              <button onClick={handleSave} className="save-btn" disabled={!ruleName}>
                Save Rule
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
