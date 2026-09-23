import { useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { Console } from "./components/Console";
import { UnpackerView } from "./views/UnpackerView";
import { ProjectsView, Project } from "./views/ProjectsView";
import { PartitionsView } from "./views/PartitionsView";
import { MagiskView } from "./views/MagiskView";
import { DebloatView } from "./views/DebloatView";
import { BuildView } from "./views/BuildView";
import { LegacyView } from "./views/LegacyView";
import { ToolsView } from "./views/ToolsView";
import { TitleBar } from "./components/TitleBar";
import "./index.css";

function App() {
  const [activeView, setActiveView] = useState("Projects");
  const [activeProject, setActiveProject] = useState<Project | null>(null);

  const handleProjectSelect = (project: Project) => {
    setActiveProject(project);
    setActiveView("Unpacker");
  };

  const renderView = () => {
    switch (activeView) {
      case "Projects":
        return <ProjectsView onProjectSelect={handleProjectSelect} />;
      case "Unpacker":
        return <UnpackerView activeProject={activeProject} />;
      case "Partitions":
        return <PartitionsView activeProject={activeProject} />;
      case "Debloat":
        return <DebloatView activeProject={activeProject} />;
      case "Magisk":
        return <MagiskView activeProject={activeProject} />;
      case "Build":
        return <BuildView activeProject={activeProject} />;
      case "Legacy":
        return <LegacyView activeProject={activeProject} />;
      case "Tools":
        return <ToolsView />;
      case "Logs":
        return <Console />;
      default:
        return <div className="md-card"><div className="md-card-title">Welcome to TuxKitchen</div></div>;
    }
  };

  return (
    <div className="window-layout">
      <TitleBar />
      <div className="app-container">
        <Sidebar activeView={activeView} setActiveView={setActiveView} />
        <div className="main-content">
          <div className="view-container" style={{ display: activeView === 'Logs' ? 'flex' : 'block', flexDirection: 'column' }}>
            <div className="view-header">
              <h2>{activeView}</h2>
            </div>
            {renderView()}
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
