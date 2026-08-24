export interface ModelConfig {
    id: string;
    name: string;
    model_path: string;
    server_path: string;
    port: number;
    ngl: number;
}

export interface RunningModelInfo extends ModelConfig {
    status: string;
    pid: number | null;
    started_at: number;
}

export interface ModelFileInfo {
    path: string;
    filename: string;
    size_bytes: number;
    size_display: string;
}

export interface PathCheckResult {
    modelValid: boolean;
    modelError: string;
    serverValid: boolean;
    serverError: string;
}
