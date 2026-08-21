declare module "*.png" {
  const image: import("next/image").StaticImageData;
  export default image;
}
