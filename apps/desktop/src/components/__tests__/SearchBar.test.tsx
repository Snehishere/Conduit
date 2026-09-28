import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import SearchBar from '../ui/SearchBar';

describe('SearchBar', () => {
  it('renders search input', () => {
    render(<SearchBar results={[]} onSearch={vi.fn()} />);
    expect(screen.getByPlaceholderText('Search...')).toBeInTheDocument();
  });

  it('calls onSearch when typing', () => {
    const onSearch = vi.fn();
    render(<SearchBar results={[]} onSearch={onSearch} />);
    const input = screen.getByPlaceholderText('Search...');
    fireEvent.change(input, { target: { value: 'test' } });
    expect(onSearch).toHaveBeenCalledWith('test');
  });

  it('renders with results when query matches', () => {
    const results = [
      { id: '1', title: 'My Phone', subtitle: 'Connected', category: 'device' as const, onClick: vi.fn() },
    ];
    render(<SearchBar results={results} onSearch={vi.fn()} />);
    const input = screen.getByPlaceholderText('Search...');
    fireEvent.change(input, { target: { value: 'phone' } });
    fireEvent.focus(input);
    expect(screen.getByText('My Phone')).toBeInTheDocument();
    expect(screen.getByText('Connected')).toBeInTheDocument();
  });

  it('shows no results message when query has no matches', () => {
    render(<SearchBar results={[]} onSearch={vi.fn()} />);
    const input = screen.getByPlaceholderText('Search...');
    fireEvent.change(input, { target: { value: 'nonexistent' } });
    fireEvent.focus(input);
    expect(screen.getByText(/No results for/)).toBeInTheDocument();
  });
});
