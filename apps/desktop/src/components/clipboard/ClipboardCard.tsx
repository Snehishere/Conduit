import React, { useState } from 'react';
import { motion } from 'motion/react';
import {
  Copy,
  Pin,
  Trash2,
  Link as LinkIcon,
  Code,
  FileText,
  Check,
  Smartphone,
  Laptop,
  Monitor,
  Key,
  MapPin,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { timeAgo } from '../../lib/time';

export interface ClipboardItem {
  id: number;
  content: string;
  mime: string;
  source_device: string;
  timestamp: number;
  pinned: boolean;
}

interface ClipboardCardProps {
  item: ClipboardItem;
  index: number;
  deviceName?: string;
  onCopy?: (id: number) => void;
  onPin?: (id: number) => void;
  onDelete?: (id: number) => void;
}

function getClipType(mime: string, content: string): 'link' | 'code' | 'text' | 'image' | 'ssh' | 'coordinates' {
  if (mime.includes('image')) return 'image';
  if (mime.includes('url') || content.match(/^https?:\/\//)) return 'link';
  if (content.match(/^ssh-/)) return 'ssh';
  if (content.match(/\d+\.\d+°\s*[NS],?\s*\d+\.\d+°\s*[EW]/)) return 'coordinates';
  if (mime.includes('code') || content.includes('\n') && (content.includes('{') || content.includes('function') || content.includes('import'))) return 'code';
  return 'text';
}

function extractDomain(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return '';
  }
}

const TYPE_CONFIG = {
  link: { 
    label: 'LINK', 
    icon: LinkIcon, 
    textClass: 'text-blue-700', 
    bgClass: 'bg-blue-500/15',
  },
  code: { 
    label: 'CODE', 
    icon: Code, 
    textClass: 'text-amber-700', 
    bgClass: 'bg-amber-500/15',
  },
  text: { 
    label: 'TEXT', 
    icon: FileText, 
    textClass: 'text-green-700', 
    bgClass: 'bg-green-500/15',
  },
  image: { 
    label: 'IMAGE', 
    icon: FileText, 
    textClass: 'text-violet-700', 
    bgClass: 'bg-violet-500/15',
  },
  ssh: { 
    label: 'SSH KEY', 
    icon: Key, 
    textClass: 'text-orange-700', 
    bgClass: 'bg-orange-500/15',
  },
  coordinates: { 
    label: 'COORDINATES', 
    icon: MapPin, 
    textClass: 'text-violet-700', 
    bgClass: 'bg-violet-500/15',
  },
};

const getDeviceIcon = (deviceName?: string) => {
  if (!deviceName) return Smartphone;
  if (deviceName.toLowerCase().includes('mac') || deviceName.toLowerCase().includes('laptop')) return Laptop;
  if (deviceName.toLowerCase().includes('desktop')) return Monitor;
  return Smartphone;
};

const ClipboardCard: React.FC<ClipboardCardProps> = ({
  item,
  index,
  deviceName,
  onCopy,
  onPin,
  onDelete,
}) => {
  const [copied, setCopied] = useState(false);
  const type = getClipType(item.mime, item.content);
  const config = TYPE_CONFIG[type];
  const DeviceIcon = getDeviceIcon(deviceName || item.source_device);

  const handleCopy = () => {
    navigator.clipboard.writeText(item.content).catch(() => {});
    setCopied(true);
    onCopy?.(item.id);
    setTimeout(() => { setCopied(false); }, 1500);
  };

  const contentLines = item.content.split('\n');
  const maxLines = type === 'link' ? 2 : Math.min(type === 'code' ? 12 : 10, Math.max(3, contentLines.length));
  const previewLines = contentLines.slice(0, maxLines);

  return (
    <>
      {/* Visually hidden live region for screen reader announcements */}
      <div className="sr-only" aria-live="polite" aria-atomic="true">
        {copied && 'Copied to clipboard'}
      </div>
    <motion.div
      className={cn(
        'break-inside-avoid mb-3 rounded-2xl p-4 transition-all group',
        'hover:shadow-card',
        'texture-ink surf-clear border border-line-1'
      )}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: index * 0.03, duration: 0.25 }}
      // Not role="button": the card contains interactive Copy/Pin/Delete
      // controls, and nesting interactive elements inside a button role is
      // an ARIA violation. Clicking the card still copies for mouse users;
      // keyboard/screen reader users use the labeled action buttons below.
      onClick={handleCopy}
    >
      {/* Header with badge and time */}
      <div className="flex items-center justify-between mb-3">
        <span 
          className={cn(
            'text-[10px] font-bold uppercase tracking-wider px-2 py-1 rounded',
            config.textClass,
            config.bgClass
          )}
        >
          {type === 'code' && item.content.includes('javascript') ? 'CODE · JAVASCRIPT' : config.label}
        </span>
        <span className="text-[11px] text-ink-3">
          {timeAgo(item.timestamp)}
        </span>
      </div>

      {/* Content preview */}
      {type === 'link' ? (
        <div className="mb-3">
          <p className={cn('text-[13px] font-medium truncate', config.textClass)}>
            {item.content}
          </p>
          <p className="text-[11px] mt-1 text-ink-3">
            {extractDomain(item.content)}
          </p>
        </div>
      ) : type === 'ssh' ? (
        <div className="mb-3">
          <pre className="text-[11px] font-mono whitespace-pre-wrap overflow-hidden leading-relaxed text-ink-2">
            {item.content.slice(0, 60)}...
          </pre>
        </div>
      ) : type === 'coordinates' ? (
        <div className="mb-3">
          <p className="text-[14px] font-mono font-medium text-ink-1">
            {item.content}
          </p>
        </div>
      ) : type === 'code' ? (
        <div className="mb-3 p-3 rounded-lg bg-fill-1">
          <pre className="text-[11px] font-mono whitespace-pre-wrap overflow-hidden leading-relaxed text-ink-2">
            {previewLines.join('\n')}
            {contentLines.length > maxLines && (
              <span className="text-ink-3">…</span>
            )}
          </pre>
        </div>
      ) : (
        <p className="text-[13px] leading-relaxed whitespace-pre-wrap mb-3 text-ink-2">
          {previewLines.join('\n')}
            {contentLines.length > maxLines && (
            <span className="text-ink-3">…</span>
          )}
        </p>
      )}

      {/* Footer with device and actions */}
      <div className="flex items-center justify-between border-t border-ink-line pt-3">
        <div className="flex items-center gap-2">
          <DeviceIcon size={12} className="text-ink-3" />
          <span className="text-[11px] text-ink-3">
            {deviceName || item.source_device}
          </span>
        </div>
        <div className="flex items-center gap-1 transition-opacity min-w-0">
          <button
            onClick={(e) => { e.stopPropagation(); handleCopy(); }}
            className="p-1.5 rounded-lg transition-colors focus-visible:opacity-100 group-hover:opacity-100 opacity-0 bg-fill-1 hover:bg-fill-2"
            aria-label={copied ? 'Copied' : 'Copy to clipboard'}
          >
            {copied ? (
              <Check size={12} className="text-success-ink" />
            ) : (
              <Copy size={12} className="text-ink-3" />
            )}
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); onPin?.(item.id); }}
            className="p-1.5 rounded-lg transition-colors focus-visible:opacity-100 group-hover:opacity-100 opacity-0 bg-fill-1 hover:bg-fill-2"
            aria-label={item.pinned ? 'Unpin item' : 'Pin item'}
          >
            <Pin size={12} className={item.pinned ? 'text-accent-ink fill-accent-ink' : 'text-ink-3'} />
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); onDelete?.(item.id); }}
            className="p-1.5 rounded-lg transition-colors focus-visible:opacity-100 group-hover:opacity-100 opacity-0 bg-fill-1 hover:bg-danger-dim"
            aria-label="Delete item"
          >
            <Trash2 size={12} className="text-ink-3" />
          </button>
        </div>
      </div>
    </motion.div>
    </>
  );
};

export default ClipboardCard;
