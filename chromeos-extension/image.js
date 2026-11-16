// Image preparation for chrome.wallpaper (runs in the service worker).

/**
 * Make the bytes chrome.wallpaper accepts (JPEG/PNG), darkened by
 * `dimPercent` (0–80) and blurred by `blurLevel` (0–10).
 * Unchanged JPEG/PNG bytes are passed straight through when neither is set.
 */
export async function prepareImage({ bytes, type }, dimPercent = 0, blurLevel = 0) {
  if (!dimPercent && !blurLevel && /^image\/(jpeg|jpg|png)\b/.test(type)) return bytes;
  // createImageBitmap applies EXIF orientation, so photos stay upright.
  const bitmap = await createImageBitmap(new Blob([bytes], { type }));
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
  const ctx = canvas.getContext("2d");
  ctx.drawImage(bitmap, 0, 0);
  if (blurLevel) {
    // Same scale as the desktop app: level 10 ≈ 30 px on a 2560 px image.
    const px = (Math.min(blurLevel, 10) * 3 * Math.max(bitmap.width, bitmap.height)) / 2560;
    // Drawing the blurred copy over the sharp one keeps the edges from
    // fading to black (the blur would otherwise pull in transparency).
    ctx.filter = `blur(${px}px)`;
    ctx.drawImage(bitmap, 0, 0);
    ctx.filter = "none";
  }
  if (dimPercent) {
    ctx.fillStyle = `rgba(0, 0, 0, ${Math.min(dimPercent, 80) / 100})`;
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.92 });
  return blob.arrayBuffer();
}
