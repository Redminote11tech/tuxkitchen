import { Box, PackageOpen, Trash2, ShieldAlert, Terminal, FolderCode, Layers, Archive, History, Wrench, FolderTree, FileCog, Scale } from "lucide-react";

interface SidebarProps {
  activeView: string;
  setActiveView: (view: string) => void;
}

export function Sidebar({ activeView, setActiveView }: SidebarProps) {
  const navItems = [
    { name: "Projects", icon: <FolderCode size={20} /> },
    { name: "Unpacker", icon: <PackageOpen size={20} /> },
    { name: "Files", icon: <FolderTree size={20} /> },
    { name: "Props", icon: <FileCog size={20} /> },
    { name: "Partitions", icon: <Layers size={20} /> },
    { name: "Debloat", icon: <Trash2 size={20} /> },
    { name: "Magisk", icon: <ShieldAlert size={20} /> },
    { name: "Compare", icon: <Scale size={20} /> },
    { name: "Build", icon: <Archive size={20} /> },
    { name: "Legacy", icon: <History size={20} /> },
    { name: "Tools", icon: <Wrench size={20} /> },
    { name: "Logs", icon: <Terminal size={20} /> },
  ];

  return (
    <div className="sidebar">
      <div className="sidebar-header">
        <Box color="var(--md-sys-color-primary)" />
        TuxKitchen
      </div>
      <ul className="nav-list">
        {navItems.map((item) => (
          <li
            key={item.name}
            className={`nav-item ${activeView === item.name ? "active" : ""}`}
            onClick={() => setActiveView(item.name)}
          >
            {item.icon}
            {item.name}
          </li>
        ))}
      </ul>
    </div>
  );
}
