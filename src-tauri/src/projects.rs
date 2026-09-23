use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: String,
}

fn get_projects_file(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let app_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|_| "Could not resolve app data directory".to_string())?;
    
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    }
    
    Ok(app_dir.join("projects.json"))
}

#[tauri::command]
pub fn get_projects(app_handle: AppHandle) -> Result<Vec<Project>, String> {
    let file_path = get_projects_file(&app_handle)?;
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    
    let contents = fs::read_to_string(file_path).map_err(|e| e.to_string())?;
    if contents.trim().is_empty() {
        return Ok(Vec::new());
    }
    
    let projects: Vec<Project> = serde_json::from_str(&contents).map_err(|e| e.to_string())?;
    Ok(projects)
}

#[tauri::command]
pub fn create_project(app_handle: AppHandle, name: String, workspace_path: String) -> Result<Project, String> {
    let mut projects = get_projects(app_handle.clone())?;
    
    let project = Project {
        id: Uuid::new_v4().to_string(),
        name,
        path: workspace_path.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
    };
    
    // Create the workspace directory if it doesn't exist
    let p = Path::new(&workspace_path);
    if !p.exists() {
        fs::create_dir_all(p).map_err(|e| format!("Failed to create workspace dir: {}", e))?;
    }
    
    projects.push(project.clone());
    
    let file_path = get_projects_file(&app_handle)?;
    let json = serde_json::to_string_pretty(&projects).map_err(|e| e.to_string())?;
    fs::write(file_path, json).map_err(|e| e.to_string())?;
    
    Ok(project)
}

#[tauri::command]
pub fn delete_project(app_handle: AppHandle, id: String) -> Result<(), String> {
    let mut projects = get_projects(app_handle.clone())?;
    projects.retain(|p| p.id != id);
    
    let file_path = get_projects_file(&app_handle)?;
    let json = serde_json::to_string_pretty(&projects).map_err(|e| e.to_string())?;
    fs::write(file_path, json).map_err(|e| e.to_string())?;
    
    Ok(())
}
