import type {Theme} from '../lib/types';

export const presets:Record<'dark'|'light',Theme> = {
  dark:{mode:'dark',canvas:'#171716',surface:'#111110',accent:'#d6b98c',ink:'#eeeeec'},
  light:{mode:'light',canvas:'#efeeea',surface:'#fdfcf9',accent:'#8b6840',ink:'#272725'},
};
