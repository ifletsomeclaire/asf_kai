rm -rf ./clean_project &&
rsync -av \
  --exclude target \
  --exclude .git \
  --exclude interner \
  --exclude tests \
  --exclude '*.log' \
  --exclude '*.redb' \
  --exclude '*.png' \
  --exclude '*.jpg' \
  --exclude '*.jpeg' \
  --exclude '*.gif' \
  --exclude '*.bmp' \
  --exclude '*.tiff' \
  --exclude '*.ico' \
  --exclude '*.webp' \
  --exclude '*.glb' \
  --exclude '*.lock' \
  --exclude '*.gltf' \
  --exclude '*.zip' \
  --exclude '*.bin' \
  --exclude '*.patch' \
  --exclude '*.sh' \
  ./ ./clean_project/