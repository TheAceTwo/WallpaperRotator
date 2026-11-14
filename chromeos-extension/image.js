// Image preparation for chrome.wallpaper (runs in the service worker).

/**
 * Make the bytes chrome.wallpaper accepts (JPEG/PNG), darkened by `dimPercent`.
 * Unchanged JPEG/PNG bytes are passed straight through when not dimming.
 */
export async function prepareImage({ bytes, type }, dimPercent) {
  if (!dimPercent && /^image\/(jpeg|jpg|png)\b/.test(type)) return bytes;
  // createImageBitmap applies EXIF orientation, so photos stay upright.
  const bitmap = await createImageBitmap(new Blob([bytes], { type }));
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const ctx = canvas.getContext("2d");
  ctx.drawImage(bitmap, 0, 0);
  if (dimPercent) {
    ctx.fillStyle = `rgba(0, 0, 0, ${Math.min(dimPercent, 80) / 100})`;
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.92 });
  return blob.arrayBuffer();
}
