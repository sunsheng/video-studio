<#
.SYNOPSIS
video-studio 安装脚本（Windows）。

.DESCRIPTION
两种用法：
  1. 直接跑（会去 GitHub 下载）：
       irm https://raw.githubusercontent.com/sunsheng/video-studio/main/scripts/install.ps1 | iex
  2. 手动下载 zip 解压后，在解压出来的目录里跑 .\install.ps1 —— 不联网，就地安装。

也可以完全不用这个脚本：把 zip 解压到任意目录直接用，程序不依赖安装位置。

.PARAMETER Prefix
安装到哪里，默认 $env:LOCALAPPDATA\video-studio

.PARAMETER Version
装哪个版本，默认最新 release

.PARAMETER Init
顺带初始化一部作品。**默认不初始化**

.PARAMETER Force
目标目录已存在时覆盖
#>
[CmdletBinding()]
param(
    [string]$Prefix = "$env:LOCALAPPDATA\video-studio",
    [string]$Version = "",
    [string]$Init = "",
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$Repo = 'sunsheng/video-studio'

function Fail($msg) { Write-Error $msg; exit 1 }

$scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
$localBinary = Join-Path $scriptDir 'studiod.exe'

if (Test-Path $localBinary) {
    Write-Host "从 $scriptDir 就地安装"
    $stage = $scriptDir
    $temp = $null
} else {
    if (-not $Version) {
        Write-Host '查询最新版本…'
        $release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
        $Version = $release.tag_name
        if (-not $Version) { Fail '查不到最新版本。用 -Version 指定，或手动下载。' }
    }
    $asset = "video-studio-$Version-windows-x86_64.zip"
    $url = "https://github.com/$Repo/releases/download/$Version/$asset"
    $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $temp -Force | Out-Null
    Write-Host "下载 $Version …"
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $temp 'pkg.zip')
    Expand-Archive -Path (Join-Path $temp 'pkg.zip') -DestinationPath (Join-Path $temp 'x') -Force
    $stage = Join-Path $temp 'x\video-studio'
    if (-not (Test-Path (Join-Path $stage 'studiod.exe'))) { Fail '包里没有 studiod.exe，压缩包可能损坏。' }
}

try {
    if ((Test-Path $Prefix) -and -not $Force) {
        if (Test-Path (Join-Path $Prefix 'studiod.exe')) {
            Write-Host "$Prefix 已有一份安装，将覆盖升级。"
        } else {
            Fail "$Prefix 已存在且不像是 video-studio 的安装目录。换个 -Prefix，或加 -Force。"
        }
    }

    New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
    Copy-Item -Path (Join-Path $stage '*') -Destination $Prefix -Recurse -Force

    $studiod = Join-Path $Prefix 'studiod.exe'
    Write-Host ''
    Write-Host "已安装到 $Prefix"
    & $studiod --version

    if (-not (Test-Path (Join-Path $Prefix '.env')) -and (Test-Path (Join-Path $Prefix '.env.example'))) {
        Write-Host "提示：按需复制 $Prefix\.env.example 为 $Prefix\.env 配置 ffmpeg 路径与 ComfyUI 节点。"
    }

    if ($Init) {
        Write-Host ''
        & $studiod init $Init
    } else {
        Write-Host ''
        Write-Host '默认没有初始化作品。要新建一部：'
        Write-Host "  $studiod init `$HOME\videos\我的第一部.studio"
    }

    Write-Host ''
    Write-Host '建议先体检一次：'
    Write-Host "  $studiod doctor"
} finally {
    if ($temp -and (Test-Path $temp)) { Remove-Item -Recurse -Force $temp }
}
