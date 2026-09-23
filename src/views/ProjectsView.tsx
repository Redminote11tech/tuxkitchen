import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderOpen, Plus, Trash2, FolderCode } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";

export interface Project {
  id: string;
  name: string;
  path: string;
  created_at: string;
}

interface ProjectsViewProps {
  onProjectSelect: (project: Project) => void;
}

export function ProjectsView({ onProjectSelect }: ProjectsViewProps) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [newProjectName, setNewProjectName] = useState("");
  const [newProjectPath, setNewProjectPath] = useState("");

  const loadProjects = async () => {
    try {
      const projs: Project[] = await invoke("get_projects");
      setProjects(projs);
    } catch (e) {
      console.error(e);
    }
  };

  useEffect(() => {
    loadProjects();
  }, []);

  const selectWorkspace = async () => {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
      });
      if (dir && typeof dir === "string") {
        setNewProjectPath(dir);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleCreate = async () => {
    if (!newProjectName || !newProjectPath) {
      alert("Please provide both a name and a workspace directory.");
      return;
    }
    try {
      await invoke("create_project", { name: newProjectName, workspacePath: newProjectPath });
      setNewProjectName("");
      setNewProjectPath("");
      loadProjects();
    } catch (e) {
      console.error(e);
      alert(`Failed to create project: ${e}`);
    }
  };

  const handleDelete = async (id: string) => {
    if (confirm("Are you sure you want to remove this project?")) {
      try {
        await invoke("delete_project", { id });
        loadProjects();
      } catch (e) {
        console.error(e);
      }
    }
  };

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="md-card-title">Create New Project</div>
        
        <div className="flex-col mt-4">
          <div>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Project Name</label>
            <div className="mt-4" style={{ marginTop: "8px" }}>
              <input 
                type="text" 
                value={newProjectName} 
                onChange={e => setNewProjectName(e.target.value)} 
                placeholder="e.g., LineageOS 20 Port" 
              />
            </div>
          </div>

          <div className="mt-4">
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: "14px" }}>Workspace Directory</label>
            <div className="flex-row mt-4" style={{ marginTop: "8px" }}>
              <input type="text" readOnly value={newProjectPath || "No workspace selected"} placeholder="Select a directory..." />
              <button className="icon-btn" onClick={selectWorkspace} title="Browse Workspace">
                <FolderOpen size={20} />
              </button>
            </div>
          </div>
          
          <div className="mt-4">
            <button className="primary" onClick={handleCreate}>
              <Plus size={16} style={{ marginRight: '8px', verticalAlign: 'middle' }} />
              Create Project
            </button>
          </div>
        </div>
      </div>

      <div className="md-card">
        <div className="md-card-title">Your Projects</div>
        {projects.length === 0 ? (
          <p style={{ color: "var(--md-sys-color-outline)" }}>No projects found. Create one above to get started.</p>
        ) : (
          <div className="project-list flex-col">
            {projects.map(proj => (
              <div key={proj.id} className="project-item flex-row" style={{
                padding: '16px', 
                backgroundColor: 'var(--md-sys-color-surface-variant)', 
                borderRadius: '8px',
                justifyContent: 'space-between'
              }}>
                <div className="flex-row">
                  <FolderCode size={32} color="var(--md-sys-color-primary)" />
                  <div className="flex-col" style={{ gap: '4px' }}>
                    <div style={{ fontWeight: 500, fontSize: '16px' }}>{proj.name}</div>
                    <div style={{ fontSize: '12px', color: 'var(--md-sys-color-outline)' }}>{proj.path}</div>
                  </div>
                </div>
                <div className="flex-row">
                  <button className="secondary" onClick={() => onProjectSelect(proj)}>Open Workspace</button>
                  <button className="icon-btn" onClick={() => handleDelete(proj.id)} title="Delete Project">
                    <Trash2 size={20} color="var(--md-sys-color-error)" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
