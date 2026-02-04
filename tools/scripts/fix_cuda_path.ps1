# Run this script as Administrator to fix CUDA_PATH
# Right-click on PowerShell -> Run as Administrator
# Then run: .\fix_cuda_path.ps1

$newCudaPath = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0"

Write-Host "Updating CUDA_PATH from v10.1 to v13.0..."
Write-Host "Current CUDA_PATH: $([System.Environment]::GetEnvironmentVariable('CUDA_PATH', 'Machine'))"

[System.Environment]::SetEnvironmentVariable("CUDA_PATH", $newCudaPath, "Machine")
[System.Environment]::SetEnvironmentVariable("CUDA_TOOLKIT_ROOT_DIR", $newCudaPath, "Machine")

Write-Host "New CUDA_PATH: $([System.Environment]::GetEnvironmentVariable('CUDA_PATH', 'Machine'))"
Write-Host ""
Write-Host "Done! Please restart your terminal/IDE for changes to take effect."
