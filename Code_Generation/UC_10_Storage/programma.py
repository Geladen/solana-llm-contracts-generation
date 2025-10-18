import re
import sys
import os

def count_comments_exclude_checks(file_path):
    with open(file_path, 'r', encoding='utf-8') as file:
        lines = file.readlines()
    
    simple_comments = 0
    doc_comments = 0
    multiline_blocks = 0
    total_lines = len(lines)
    
    print(f"📋 COMMENTI TROVATI in {os.path.basename(file_path)} (esclusi ///check):")
    
    i = 0
    while i < len(lines):
        line = lines[i].rstrip()
        
        # ESCLUDI ESPLICITAMENTE i ///check
        if re.search(r'///\s*CHECK', line, re.IGNORECASE):
            print(f"Riga {i+1:3d} [ESCLUSO - CHECK]: {line}")
            i += 1
            continue
            
        # Commenti multilinea /* */
        if '/*' in line:
            multiline_start = i
            multiline_content = []
            multiline_content.append(line)
            i += 1
            while i < len(lines) and '*/' not in lines[i]:
                multiline_content.append(lines[i].rstrip())
                i += 1
            if i < len(lines) and '*/' in lines[i]:
                multiline_content.append(lines[i].rstrip())
                multiline_blocks += 1
                print(f"Riga {multiline_start+1:3d} [MULTILINEA]: {' '.join(multiline_content)}")
        
        # Commenti doc /// (esclusi i check)
        elif line.strip().startswith('///') and not re.search(r'///\s*CHECK', line, re.IGNORECASE):
            doc_comments += 1
            print(f"Riga {i+1:3d} [DOC]: {line}")
        
        # Commenti semplici // (inclusi quelli inline)
        elif '//' in line and not line.strip().startswith('///'):
            # Verifica che non sia un falso positivo in una stringa
            if not is_comment_in_string(line):
                simple_comments += 1
                # Distingui tra commenti a inizio riga e inline
                if line.strip().startswith('//'):
                    print(f"Riga {i+1:3d} [SEMPLICE]: {line}")
                else:
                    print(f"Riga {i+1:3d} [INLINE]: {line}")
        
        i += 1
    
    total_comments = simple_comments + doc_comments + multiline_blocks
    
    # Calcola il rapporto commenti/righe
    comment_ratio = (total_comments / total_lines) if total_lines > 0 else 0
    
    print(f"\n📊 STATISTICHE FINALI:")
    print(f"  Righe totali del file: {total_lines}")
    print(f"  Commenti semplici //: {simple_comments}")
    print(f"  Doc comments ///: {doc_comments}")
    print(f"  Blocchi multilinea: {multiline_blocks}")
    print(f"  TOTALE COMMENTI: {total_comments}")
    print(f"  RAPPORTO COMMENTI/RIGHE: {total_comments}/{total_lines} = {comment_ratio:.2f}%")
    
    return total_comments, total_lines, comment_ratio

def is_comment_in_string(line):
    """Controlla se il // è dentro una stringa"""
    before_comment = line.split('//')[0]
    
    # Conta i doppi apici prima del commento
    quote_count = before_comment.count('"')
    
    # Se il numero di doppi apici è dispari, siamo dentro una stringa
    return quote_count % 2 == 1

def main():
    if len(sys.argv) != 2:
        print("Usage: python programma.py <file.rs>")
        print("Example: python programma.py mio_file.rs")
        sys.exit(1)
    
    file_path = sys.argv[1]
    
    if not os.path.exists(file_path):
        print(f"Errore: Il file '{file_path}' non esiste!")
        sys.exit(1)
    
    if not file_path.endswith('.rs'):
        print(f"Avviso: Il file '{file_path}' non sembra un file Rust (.rs)")
    
    total_comments, total_lines, comment_ratio = count_comments_exclude_checks(file_path)

if __name__ == "__main__":
    main()
