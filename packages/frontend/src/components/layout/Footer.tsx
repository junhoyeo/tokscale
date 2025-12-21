import Image from 'next/image';

export function Footer() {
  return (
    <div className="w-full max-w-7xl mx-auto px-6">
      <footer className="relative w-full h-[436px] rounded-[20px] overflow-hidden bg-gradient-to-b from-black to-[#10121C]">
        <div className="absolute top-[52px] left-[60px] flex flex-col gap-[21px] z-10">
          <div className="relative w-[107.29px] h-[100px]">
            <Image
              src="/assets/footer-logo-icon.png"
              alt="Tokscale Icon"
              fill
              className="object-contain"
            />
          </div>

          <a 
            href="https://tokscale.ai" 
            target="_blank" 
            rel="noopener noreferrer" 
            className="hover:opacity-80 transition-opacity block"
          >
            <Image
              src="/assets/footer-logo.svg"
              alt="Tokscale"
              width={184}
              height={21}
              className="w-[184px] h-[21px]"
            />
          </a>

          <div className="w-[74px] h-[2px] bg-[#0073FF]" />

          <div className="flex flex-col gap-[4px]">
            <p className="text-[#0073FF] text-[18px] font-medium leading-tight font-sans">
              © 2025 Tokscale. All rights reserved.
            </p>
            <a 
              href="https://github.com/junhoyeo/tokscale" 
              target="_blank" 
              rel="noopener noreferrer"
              className="text-[#3D526C] text-[18px] font-medium leading-tight font-sans hover:text-[#0073FF] transition-colors"
            >
              github.com/junhoyeo/tokscale
            </a>
          </div>
        </div>

        <div className="absolute right-0 top-[-135px] pointer-events-none select-none">
          <Image
            src="/assets/footer-globe.svg"
            alt=""
            width={435}
            height={435}
            className="w-[435px] h-auto animate-[spin_60s_linear_infinite]"
            priority
          />
        </div>
      </footer>
    </div>
  );
}
