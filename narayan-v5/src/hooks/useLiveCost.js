import { useState, useCallback } from 'react';

export function useLiveCost() {
  const [totalCost, setTotalCost] = useState(0);

  const addCost = useCallback((amount) => {
    if (typeof amount === 'number' && amount > 0) {
      setTotalCost(prev => prev + amount);
    }
  }, []);

  const reset = useCallback(() => setTotalCost(0), []);

  return { totalCost, addCost, reset };
}
