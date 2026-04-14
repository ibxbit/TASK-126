import { invoke } from "@tauri-apps/api/core";

export interface LoginResponse {
  user_id: string;
  username: string;
  role: string;
  tenant_ids: string[];
}

export async function login(username: string, password: string): Promise<LoginResponse> {
  return invoke<LoginResponse>("cmd_login", { username, password });
}

export async function logout(): Promise<void> {
  await invoke("cmd_logout");
}

export async function currentUser(): Promise<LoginResponse | null> {
  return invoke<LoginResponse | null>("cmd_current_user");
}
