/// Zera `buf` de um jeito que o otimizador não pode descartar como "morto".
/// Sem isso a chave copiada para a pilha sobrevive depois do uso.
pub fn wipe<T: Copy + Default>(buf: &mut [T]) {
    buf.fill(T::default());
    core::hint::black_box(buf);
}

#[cfg(test)]
mod tests {
    use super::wipe;

    #[test]
    fn wipe_zeroes_every_element() {
        let mut bytes = [0xA5_u8; 37];
        wipe(&mut bytes);
        assert_eq!(bytes, [0; 37]);
        let mut words = [u64::MAX; 8];
        wipe(&mut words);
        assert_eq!(words, [0; 8]);
    }

    #[test]
    fn wipe_accepts_an_empty_slice() {
        wipe::<u8>(&mut []);
    }
}
