import { useState, useEffect, useCallback } from 'react';
import { workspace as workspaceApi } from '../api';

export function useWorkspaceFiles(agentId, agentStatus) {
  const [files, setFiles] = useState([]);
  const [selectedFile, setSelectedFile] = useState({ path: null, content: null, loading: false });
  const [isCollapsed, setIsCollapsed] = useState(() => localStorage.getItem('narayan_workspace_collapsed') === 'true');
  const [newFiles, setNewFiles] = useState(new Set());

  const isTerminal = agentStatus === 'completed' || agentStatus === 'failed';

  const fetchTree = useCallback(async () => {
    if (!agentId) return;
    try {
      const data = await workspaceApi.tree(agentId);
      const tree = data.files || data.tree || data || [];
      setFiles(prev => {
        const prevPaths = new Set();
        const flatten = items => items.forEach(f => { prevPaths.add(f.path); if (f.children) flatten(f.children); });
        flatten(prev);
        const added = new Set();
        const flattenNew = items => items.forEach(f => { if (!prevPaths.has(f.path)) added.add(f.path); if (f.children) flattenNew(f.children); });
        flattenNew(tree);
        if (added.size > 0) {
          setNewFiles(added);
          setTimeout(() => setNewFiles(new Set()), 3000);
          if (isCollapsed && prev.length === 0) setIsCollapsed(false);
        }
        return tree;
      });
    } catch {}
  }, [agentId, isCollapsed]);

  useEffect(() => {
    fetchTree();
    if (isTerminal) return;
    const iv = setInterval(fetchTree, 3000);
    return () => clearInterval(iv);
  }, [agentId, isTerminal, fetchTree]);

  async function selectFile(path) {
    setSelectedFile({ path, content: null, loading: true });
    try {
      const data = await workspaceApi.file(agentId, path);
      setSelectedFile({ path, content: typeof data === 'string' ? data : JSON.stringify(data, null, 2), loading: false });
    } catch {
      setSelectedFile({ path, content: 'Failed to load', loading: false });
    }
  }

  function toggleCollapse() {
    const next = !isCollapsed;
    setIsCollapsed(next);
    localStorage.setItem('narayan_workspace_collapsed', String(next));
  }

  return { files, selectedFile, selectFile, isCollapsed, toggleCollapse, newFiles };
}
