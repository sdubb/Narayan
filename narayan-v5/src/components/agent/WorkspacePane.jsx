import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import { PanelRightClose, PanelRightOpen, FileText, FolderOpen, Download, Image, Code, File } from 'lucide-react';
import { workspace as workspaceApi } from '../../api';

const ICON_MAP = {
  md: FileText, txt: FileText, csv: Code, json: Code,
  png: Image, jpg: Image, jpeg: Image, gif: Image, svg: Image,
};

function fileIcon(name) {
  const ext = name.split('.').pop()?.toLowerCase();
  return ICON_MAP[ext] || File;
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function FileNode({ file, depth = 0, onSelect, selectedPath, newFiles }) {
  const [open, setOpen] = useState(true);
  const isDir = Boolean(file.isDir ?? file.is_dir);
  const Icon = isDir ? FolderOpen : fileIcon(file.name);
  const isNew = newFiles.has(file.path);
  const isSelected = file.path === selectedPath;

  if (isDir) {
    return (
      <div>
        <button onClick={() => setOpen(o => !o)}
          className="w-full flex items-center gap-2 px-2 py-1.5 hover:bg-bg-hover rounded transition-colors"
          style={{ paddingLeft: `${8 + depth * 12}px` }}>
          <FolderOpen size={13} className="text-tx-3 shrink-0" />
          <span className="text-xs text-tx-2 flex-1 text-left truncate">{file.name}/</span>
        </button>
        {open && file.children?.map(child => (
          <FileNode key={child.path} file={child} depth={depth + 1} onSelect={onSelect} selectedPath={selectedPath} newFiles={newFiles} />
        ))}
      </div>
    );
  }

  return (
    <motion.button
      onClick={() => onSelect(file.path)}
      className={clsx(
        'w-full flex items-center gap-2 px-2 py-1.5 rounded transition-colors',
        isSelected ? 'bg-accent-soft border border-accent/20' : 'hover:bg-bg-hover',
      )}
      style={{ paddingLeft: `${8 + depth * 12}px` }}
      initial={isNew ? { opacity: 0, x: -8 } : false}
      animate={{ opacity: 1, x: 0 }}
    >
      {isNew && <span className="size-1.5 rounded-full bg-accent animate-pulse-dot shrink-0" />}
      <Icon size={13} className="text-tx-3 shrink-0" />
      <span className="text-xs text-tx-1 flex-1 text-left truncate">{file.name}</span>
      {file.size != null && <span className="text-[10px] font-mono text-tx-4 shrink-0">{formatSize(file.size)}</span>}
    </motion.button>
  );
}

export default function WorkspacePane({ agentId, agentStatus }) {
  const [files, setFiles] = useState([]);
  const [selectedPath, setSelectedPath] = useState(null);
  const [content, setContent] = useState(null);
  const [loadingContent, setLoadingContent] = useState(false);
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem('narayan_workspace_collapsed') === 'true');
  const [newFiles, setNewFiles] = useState(new Set());
  const [prevPaths, setPrevPaths] = useState(new Set());

  const isTerminal = agentStatus === 'completed' || agentStatus === 'failed';

  const fetchFiles = useCallback(async () => {
    if (!agentId) return;
    try {
      const data = await workspaceApi.tree(agentId);
      const tree = data.files || data.tree || data || [];
      setFiles(tree);
      const currentPaths = new Set();
      const flatten = (items) => items.forEach(f => { currentPaths.add(f.path); if (f.children) flatten(f.children); });
      flatten(tree);
      setPrevPaths(prev => {
        const added = new Set();
        currentPaths.forEach(p => { if (!prev.has(p)) added.add(p); });
        if (added.size > 0) {
          setNewFiles(added);
          setTimeout(() => setNewFiles(new Set()), 3000);
          if (collapsed && prev.size === 0) setCollapsed(false);
        }
        return currentPaths;
      });
    } catch {}
  }, [agentId]);

  useEffect(() => {
    fetchFiles();
    if (isTerminal) return;
    const iv = setInterval(fetchFiles, 3000);
    return () => clearInterval(iv);
  }, [agentId, isTerminal, fetchFiles]);

  async function selectFile(path) {
    setSelectedPath(path);
    setLoadingContent(true);
    try {
      const data = await workspaceApi.file(agentId, path);
      setContent(typeof data === 'string' ? data : JSON.stringify(data, null, 2));
    } catch { setContent('Failed to load file'); }
    finally { setLoadingContent(false); }
  }

  async function downloadSelectedFile() {
    if (!agentId || !selectedPath) return;
    try {
      const blob = await workspaceApi.download(agentId, selectedPath);
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = selectedPath.split('/').pop() || 'workspace-file';
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch {}
  }

  function toggleCollapse() {
    const next = !collapsed;
    setCollapsed(next);
    localStorage.setItem('narayan_workspace_collapsed', String(next));
  }

  if (collapsed) {
    return (
      <button onClick={toggleCollapse} className="p-2 border-l border-border bg-bg-card hover:bg-bg-hover transition-colors" title="Show workspace">
        <PanelRightOpen size={16} className="text-tx-3" />
      </button>
    );
  }

  return (
    <div className="w-80 flex flex-col border-l border-border bg-bg-card shrink-0">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2.5 border-b border-border">
        <span className="text-xs font-semibold text-tx-2">Workspace</span>
        <button onClick={toggleCollapse} className="p-1 rounded text-tx-4 hover:text-tx-2 transition-colors">
          <PanelRightClose size={14} />
        </button>
      </div>

      {/* File tree */}
      <div className="flex-1 overflow-y-auto py-1">
        {files.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 text-center px-4">
            <File size={20} className="text-tx-4 mb-2" />
            <p className="text-xs text-tx-3">No files yet</p>
          </div>
        ) : (
          files.map(f => <FileNode key={f.path} file={f} onSelect={selectFile} selectedPath={selectedPath} newFiles={newFiles} />)
        )}
      </div>

      {/* Preview */}
      <AnimatePresence>
        {selectedPath && (
          <motion.div
            className="border-t border-border flex flex-col max-h-64"
            initial={{ height: 0 }} animate={{ height: 'auto' }} exit={{ height: 0 }}
          >
            <div className="flex items-center justify-between px-3 py-2 border-b border-border/60">
              <span className="text-[11px] font-mono text-tx-3 truncate flex-1">{selectedPath}</span>
              <button onClick={downloadSelectedFile} className="p-1 text-tx-4 hover:text-tx-2 transition-colors" title="Download">
                <Download size={12} />
              </button>
            </div>
            <div className="flex-1 overflow-y-auto px-3 py-2">
              {loadingContent ? (
                <p className="text-xs text-tx-4">Loading...</p>
              ) : (
                <pre className="text-[11px] font-mono text-tx-2 whitespace-pre-wrap break-words leading-relaxed">{content}</pre>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
